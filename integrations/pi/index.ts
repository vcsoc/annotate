// Annotate integration for Pi (installed by the Annotate app; do not edit here).
//
// Registers this Pi session as a live Annotate destination and adds /annotate.
// The Annotate Console POSTs a saved batch to /deliver; it is sent to the agent
// as a user message with every annotated screenshot attached.
import { spawn } from 'node:child_process';
import { randomBytes, randomUUID, timingSafeEqual } from 'node:crypto';
import { mkdirSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { homedir } from 'node:os';
import { join } from 'node:path';
import type { ExtensionAPI, ExtensionContext } from '@earendil-works/pi-coding-agent';

const BIN = process.env.ANNOTATE_BIN || '__ANNOTATE_BIN__';
const MAX_BODY = 24 * 1024 * 1024;
const home = () => process.env.ANNOTATE_HOME || join(homedir(), '.annotate');
const registry = () => process.env.ANNOTATE_REGISTRY_DIR || join(home(), 'sessions');
const MODES = [{ id: 'followUp', label: 'Follow-up' }, { id: 'steer', label: 'Queued (steering)' }];

function runAnnotate(args: string[]) {
  return new Promise<void>((resolve, reject) => {
    const child = spawn(BIN, args, { stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true });
    let errors = '';
    child.stderr?.on('data', chunk => { errors = (errors + chunk).slice(-2000); });
    child.once('error', error => reject(new Error(`Cannot run ${BIN}: ${error.message}. Reinstall the Pi integration from Annotate → Settings → Integrations.`)));
    child.once('close', code => (code ? reject(new Error(errors.trim() || `annotate exited with ${code}`)) : resolve()));
  });
}

export default function annotate(pi: ExtensionAPI) {
  const id = randomUUID();
  let current: ExtensionContext | undefined;
  let server: http.Server | undefined;
  let recordFile: string | undefined;
  let starting: Promise<void> | undefined;
  const accepted = new Map<string, unknown>();
  let pending = false;

  async function register(ctx: ExtensionContext) {
    current = ctx;
    if (server) return;
    starting ??= (async () => {
      const token = randomBytes(32).toString('hex');
      const project = ctx.cwd, session = ctx.sessionManager.getSessionId();
      const expected = Buffer.from(`Bearer ${token}`);
      const srv = http.createServer(async (req, res) => {
        const reply = (status: number, body: unknown) => { if (!res.destroyed) { res.writeHead(status, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' }); res.end(JSON.stringify(body)); } };
        try {
          const auth = Buffer.from(String(req.headers.authorization || ''));
          const port = (srv.address() as any)?.port;
          if (req.headers.origin || req.headers.host !== `127.0.0.1:${port}` || auth.length !== expected.length || !timingSafeEqual(auth, expected)) return reply(401, { error: 'Unauthorized companion' });
          if (req.method === 'GET' && req.url === '/session') {
            return reply(200, { targetId: id, harness: 'pi', harnessName: 'Pi', project, session, name: (pi as any).getSessionName?.() || '', deliveryModes: MODES, protocol: 1 });
          }
          if (req.method !== 'POST' || !['/deliver', '/status'].includes(req.url || '')) return reply(404, { error: 'Not found' });
          const chunks: Buffer[] = []; let size = 0;
          for await (const chunk of req) { size += chunk.length; if (size > MAX_BODY) return reply(413, { error: 'Batch too large' }); chunks.push(chunk); }
          const value = JSON.parse(Buffer.concat(chunks).toString('utf8'));
          if (req.url === '/status') { current?.ui.notify(`Annotate: ${String(value.message || '').slice(0, 500)}`, 'info'); return reply(200, { ok: true }); }
          if (value?.protocol !== 1 || typeof value.id !== 'string' || !Array.isArray(value.items) || !value.items.length || value.items.length > 12) return reply(400, { error: 'Unsupported annotation batch. Update Annotate.' });
          if (accepted.has(value.id)) return reply(200, accepted.get(value.id));
          if (pending) return reply(409, { error: 'A batch is already being sent; retry shortly' });
          if (!current?.model?.input?.includes('image')) return reply(400, { error: 'Select a vision-capable model in this Pi session, then Send again. Annotations remain in the Console.' });
          const deliverAs = value.deliverAs === 'steer' ? 'steer' : 'followUp';
          pending = true;
          try {
            const content: any[] = [{ type: 'text', text: String(value.text) }];
            for (const item of value.items) {
              content.push({ type: 'text', text: String(item.text) });
              content.push({ type: 'image', data: String(item.image), mimeType: 'image/png' });
            }
            await pi.sendUserMessage(content, { deliverAs });
            const result = { ok: true, count: value.items.length, savedTo: value.savedTo, deliverAs };
            accepted.set(value.id, result);
            if (accepted.size > 100) accepted.delete(accepted.keys().next().value as string);
            return reply(200, result);
          } finally { pending = false; }
        } catch (error) { reply(400, { error: error instanceof Error ? error.message : 'Invalid annotation request' }); }
      });
      srv.requestTimeout = 60000;
      await new Promise<void>((resolve, reject) => { srv.once('error', reject); srv.listen(0, '127.0.0.1', () => resolve()); });
      const port = (srv.address() as any).port;
      const dir = registry();
      mkdirSync(dir, { recursive: true, mode: 0o700 });
      const file = join(dir, `${id}.json`), temp = `${file}.${randomUUID()}`;
      writeFileSync(temp, JSON.stringify({ id, url: `http://127.0.0.1:${port}`, token, harness: 'pi', harnessName: 'Pi', project, session, pid: process.pid, protocol: 1 }), { mode: 0o600, flag: 'wx' });
      renameSync(temp, file);
      server = srv; recordFile = file;
    })().finally(() => { starting = undefined; });
    await starting;
  }

  async function shutdown() {
    const file = recordFile; recordFile = undefined;
    if (file) { try { unlinkSync(file); } catch { /* already removed */ } }
    const srv = server; server = undefined;
    if (srv) await new Promise<void>(resolve => { srv.close(() => resolve()); srv.closeAllConnections?.(); });
  }

  pi.registerCommand('annotate', {
    description: 'Open Annotate Console aimed at this session: whole-app screenshots, highlighted areas, notes',
    handler: async (args, ctx) => {
      const action = args.trim().toLowerCase();
      if (action === 'help') return ctx.ui.notify('/annotate opens (or retargets) the Annotate Console to this Pi session, keeping drafts. Capture → mark an app area → note → ✓. Choose Follow-up or Queued (steering) and Send. /annotate stop closes the Console.', 'info');
      if (action && action !== 'stop') return ctx.ui.notify('Usage: /annotate [help|stop]', 'warning');
      try {
        if (action === 'stop') { await runAnnotate(['quit']); return; }
        await register(ctx);
        await runAnnotate(['open', '--target', id]);
        ctx.ui.notify('Annotate Console is aimed at this Pi session. Nothing is sent until you press Send.', 'info');
      } catch (error) { ctx.ui.notify(error instanceof Error ? error.message : String(error), 'error'); }
    },
  });
  pi.on('session_start', async (_event, ctx) => {
    if (!ctx.hasUI) return;
    try { await register(ctx); }
    catch (error) { ctx.ui.notify(`Annotate destination registration failed: ${error instanceof Error ? error.message : error}`, 'warning'); }
  });
  pi.on('model_select', (_event, ctx) => { if (current) current = ctx; });
  pi.on('session_shutdown', shutdown);
}
