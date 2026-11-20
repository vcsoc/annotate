// Annotate integration for OpenCode (installed by the Annotate app; do not edit here).
//
// Registers this OpenCode instance as a live Annotate destination. Batches sent
// from the Annotate Console are prompted into the most recently active session
// with every annotated screenshot attached as an image part.
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import { mkdirSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { homedir } from 'node:os';
import { join } from 'node:path';

const MAX_BODY = 24 * 1024 * 1024;
const home = () => process.env.ANNOTATE_HOME || join(homedir(), '.annotate');
const registry = () => process.env.ANNOTATE_REGISTRY_DIR || join(home(), 'sessions');

export const AnnotatePlugin = async ({ client, directory, worktree }) => {
  const id = randomUUID(), token = randomBytes(32).toString('hex');
  const project = worktree || directory || process.cwd();
  const children = new Set();
  const accepted = new Map();
  let sessionID, pending = false, recordFile;

  async function latestSession() {
    if (sessionID) return sessionID;
    try {
      const result = await client.session.list();
      const list = (result?.data || result || []).filter(s => !s.parentID);
      list.sort((a, b) => (b.time?.updated || 0) - (a.time?.updated || 0));
      return list[0]?.id;
    } catch { return undefined; }
  }

  async function prompt(target, parts) {
    const session = client.session;
    // Support both SDK shapes ({ path, body } and the flat v2 form). Do not wait for the reply.
    if (typeof session.promptAsync === 'function') {
      try { return await session.promptAsync({ path: { id: target }, body: { parts } }); }
      catch { return await session.promptAsync({ sessionID: target, parts }); }
    }
    const run = session.prompt({ path: { id: target }, body: { parts } });
    run.catch?.(() => session.prompt({ sessionID: target, parts }).catch(() => {}));
    await Promise.race([run, new Promise(resolve => setTimeout(resolve, 1500))]);
  }

  const expected = Buffer.from(`Bearer ${token}`);
  const server = http.createServer(async (req, res) => {
    const reply = (status, body) => { if (!res.destroyed) { res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' }); res.end(JSON.stringify(body)); } };
    try {
      const auth = Buffer.from(String(req.headers.authorization || ''));
      if (req.headers.origin || req.headers.host !== `127.0.0.1:${server.address()?.port}` || auth.length !== expected.length || !timingSafeEqual(auth, expected)) return reply(401, { error: 'Unauthorized companion' });
      if (req.method === 'GET' && req.url === '/session') {
        return reply(200, { targetId: id, harness: 'opencode', harnessName: 'OpenCode', project, session: id, name: '', deliveryModes: [{ id: 'prompt', label: 'Prompt the active session' }], protocol: 1 });
      }
      if (req.method !== 'POST' || !['/deliver', '/status'].includes(req.url)) return reply(404, { error: 'Not found' });
      const chunks = []; let size = 0;
      for await (const chunk of req) { size += chunk.length; if (size > MAX_BODY) return reply(413, { error: 'Batch too large' }); chunks.push(chunk); }
      const value = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      if (req.url === '/status') return reply(200, { ok: true });
      if (value?.protocol !== 1 || typeof value.id !== 'string' || !Array.isArray(value.items) || !value.items.length || value.items.length > 12) return reply(400, { error: 'Unsupported annotation batch. Update Annotate.' });
      if (accepted.has(value.id)) return reply(200, accepted.get(value.id));
      if (pending) return reply(409, { error: 'A batch is already being sent; retry shortly' });
      pending = true;
      try {
        const target = await latestSession();
        if (!target) return reply(409, { error: 'Open or start an OpenCode session first, then Send again.' });
        const parts = [{ type: 'text', text: String(value.text) }];
        for (const item of value.items) {
          parts.push({ type: 'text', text: String(item.text) });
          parts.push({ type: 'file', mime: 'image/png', filename: `annotation-${item.index}.png`, url: `data:image/png;base64,${item.image}` });
        }
        await prompt(target, parts);
        const result = { ok: true, count: value.items.length, savedTo: value.savedTo, deliverAs: 'prompt' };
        accepted.set(value.id, result);
        if (accepted.size > 100) accepted.delete(accepted.keys().next().value);
        return reply(200, result);
      } finally { pending = false; }
    } catch (error) { reply(400, { error: error?.message || 'Invalid annotation request' }); }
  });
  server.requestTimeout = 60000;
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  server.unref?.();
  try {
    const dir = registry();
    mkdirSync(dir, { recursive: true, mode: 0o700 });
    const file = join(dir, `${id}.json`), temp = `${file}.${randomUUID()}`;
    writeFileSync(temp, JSON.stringify({ id, url: `http://127.0.0.1:${server.address().port}`, token, harness: 'opencode', harnessName: 'OpenCode', project, session: id, pid: process.pid, protocol: 1 }), { mode: 0o600, flag: 'wx' });
    renameSync(temp, file);
    recordFile = file;
  } catch { /* Registration is best effort; OpenCode keeps working. */ }
  const cleanup = () => { if (recordFile) { try { unlinkSync(recordFile); } catch { /* gone */ } recordFile = undefined; } };
  // Never take over OpenCode's signal handling; stale records fail the liveness check anyway.
  process.once('exit', cleanup);

  return {
    event: async ({ event }) => {
      const props = event?.properties || {};
      const info = props.info || {};
      if (event?.type === 'session.created' || event?.type === 'session.updated') {
        if (info.parentID) children.add(info.id);
        else if (info.id) sessionID = info.id;
        return;
      }
      const candidate = props.sessionID || info.sessionID;
      if (candidate && !children.has(candidate) && (event?.type === 'message.updated' || event?.type === 'session.status' || event?.type === 'session.idle')) sessionID = candidate;
    },
  };
};
