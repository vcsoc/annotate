// Drives the Pi extension and the OpenCode plugin with fake harness APIs and
// checks the Annotate bridge protocol end to end: registration, liveness,
// delivery (text + images) and cleanup.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readdirSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const home = mkdtempSync(join(tmpdir(), 'annotate-int-'));
process.env.ANNOTATE_HOME = home;
const registry = join(home, 'sessions');
const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';
const pkg = id => ({ protocol: 1, id, deliverAs: 'steer', project: '/p', savedTo: '/p/.annotate/' + id, createdAt: '', text: 'UI annotation batch (1 item).', items: [{ index: 1, source: 'App', comment: 'Fix', kind: 'rectangle', points: [[0, 0], [1, 1]], marks: [], width: 1, height: 1, file: '/p/1.png', text: 'Annotation 1 — App\nComment: Fix', image: PNG }] });
const record = () => JSON.parse(readFileSync(join(registry, readdirSync(registry).find(f => f.endsWith('.json'))), 'utf8'));
const call = (r, path, body) => fetch(r.url + path, { method: body ? 'POST' : 'GET', headers: { Authorization: `Bearer ${r.token}`, ...(body && { 'Content-Type': 'application/json' }) }, body: body && JSON.stringify(body) }).then(async res => ({ status: res.status, body: await res.json() }));

test('Pi extension registers, delivers with images and cleans up', async () => {
  const { default: annotate } = await import('../integrations/pi/index.ts');
  const handlers = {}, commands = {}, sent = [], notes = [];
  const pi = { registerCommand: (name, spec) => { commands[name] = spec; }, on: (event, fn) => { handlers[event] = fn; }, sendUserMessage: async (content, options) => { sent.push({ content, options }); }, getSessionName: () => 'demo' };
  annotate(pi);
  assert.ok(commands.annotate, '/annotate is registered');
  const ctx = { hasUI: true, cwd: '/work/project', sessionManager: { getSessionId: () => 'session-1' }, model: { input: ['text', 'image'] }, ui: { notify: (m, l) => notes.push([m, l]), setStatus() {} } };
  await handlers.session_start({}, ctx);
  const r = record();
  assert.equal(r.harness, 'pi'); assert.equal(r.project, '/work/project'); assert.match(r.token, /^[a-f0-9]{64}$/);
  const status = await call(r, '/session');
  assert.equal(status.body.targetId, r.id); assert.equal(status.body.harnessName, 'Pi');
  assert.deepEqual(status.body.deliveryModes.map(m => m.id), ['followUp', 'steer']);
  assert.equal((await fetch(r.url + '/session')).status, 401, 'token required');
  const delivered = await call(r, '/deliver', pkg('batch-0001'));
  assert.equal(delivered.status, 200); assert.equal(delivered.body.deliverAs, 'steer');
  assert.equal(sent.length, 1); assert.equal(sent[0].options.deliverAs, 'steer');
  assert.deepEqual(sent[0].content.map(c => c.type), ['text', 'text', 'image']);
  await call(r, '/deliver', pkg('batch-0001'));
  assert.equal(sent.length, 1, 'retries are idempotent');
  ctx.model = { input: ['text'] };
  assert.match((await call(r, '/deliver', pkg('batch-0002'))).body.error, /vision-capable/);
  await handlers.session_shutdown();
  assert.equal(readdirSync(registry).filter(f => f.endsWith('.json')).length, 0, 'record removed');
});

test('OpenCode plugin prompts the active session with image parts', async () => {
  const { AnnotatePlugin } = await import('../integrations/opencode/annotate.js');
  const prompts = [];
  const client = { session: { list: async () => ({ data: [] }), promptAsync: async args => { prompts.push(args); return {}; } } };
  const hooks = await AnnotatePlugin({ client, directory: '/work/oc', worktree: '/work/oc' });
  const r = record();
  assert.equal(r.harness, 'opencode'); assert.equal(r.project, '/work/oc');
  assert.equal((await call(r, '/deliver', pkg('batch-0003'))).status, 409, 'no session yet');
  await hooks.event({ event: { type: 'session.created', properties: { info: { id: 'ses_child', parentID: 'ses_root' } } } });
  await hooks.event({ event: { type: 'session.updated', properties: { info: { id: 'ses_root' } } } });
  await hooks.event({ event: { type: 'message.updated', properties: { info: { sessionID: 'ses_child' } } } });
  const delivered = await call(r, '/deliver', pkg('batch-0004'));
  assert.equal(delivered.status, 200);
  assert.equal(prompts.length, 1);
  assert.equal(prompts[0].path.id, 'ses_root', 'child sessions are ignored');
  assert.deepEqual(prompts[0].body.parts.map(p => p.type), ['text', 'text', 'file']);
  assert.match(prompts[0].body.parts[2].url, /^data:image\/png;base64,/);
});

test.after(() => { if (existsSync(home)) rmSync(home, { recursive: true, force: true }); setTimeout(() => process.exit(0), 50).unref(); });
