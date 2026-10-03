#!/usr/bin/env node
// Real public-surface acceptance, reused by Cargo and the installed-package smoke.
// All writes (including external worktree removal) stay in our mkdtemp fixture.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const binary = path.resolve(process.argv[2] ?? 'target/debug/work');
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'work-beta-'));
const covered = new Set();
const samples = new Map();
const session = { namespace: 'fixture / α', id: 'external session : 1' };
const text = '\r\n# Result α\n---\nOpaque context {{literal}}\n\n';

function git(cwd, ...args) {
  const p = spawnSync('git', ['-C', cwd, ...args], { encoding: 'utf8', timeout: 30000 });
  assert.equal(p.status, 0, p.stderr);
  return p.stdout.trim();
}
function fixture(name) {
  const root = path.join(scratch, name);
  fs.mkdirSync(path.join(root, '.work/items'), { recursive: true });
  git(root, 'init', '-q', '--initial-branch=main');
  return root;
}
function linked(root) {
  git(root, 'add', '.work');
  git(root, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture');
  const result = path.join(root, 'linked');
  git(root, 'worktree', 'add', '-qb', 'fixture-linked', result);
  return result;
}
function command(name, a) {
  const flags = (mapping) => Object.entries(mapping).flatMap(([key, flag]) => {
    if (!(key in a)) return [];
    const v = a[key];
    if (typeof v === 'boolean') return v ? [flag] : [];
    if (Array.isArray(v)) return v.flatMap(x => [flag, typeof x === 'object' ? JSON.stringify(x) : String(x)]);
    return [flag, String(v)];
  });
  const auth = () => flags({ authorization: '--authorize' });
  const scope = () => flags({ root: '--root', run_id: '--run', labels_all: '--label', priority_max: '--priority-max', persistence: '--persistence', view: '--view' });
  const sess = () => ['--session-namespace', a.session.namespace, '--session-id', a.session.id];
  if (name === 'discover') return ['discover'];
  if (name === 'item_create') return ['item', 'create', ...flags({ title: '--title', body: '--body', completion: '--completion', priority: '--priority', labels: '--label', parent: '--parent', model: '--model', thinking: '--thinking' }), ...auth()];
  if (['item_list', 'item_ready', 'item_diagnose'].includes(name)) return ['item', name.slice(5), ...scope()];
  if (name === 'item_inspect_raw') return ['item', 'inspect', a.id, '--raw'];
  if (name === 'item_inspect') return ['item', 'inspect', a.id, ...flags({ view: '--view' })];
  if (name === 'item_repair') return ['item', 'repair', a.id, '--source', '-', ...auth()];
  if (name === 'item_update') return ['item', 'update', a.id, ...flags({ title: '--title', priority: '--priority', labels: '--label', model: '--model', thinking: '--thinking' }), ...auth()];
  if (name === 'item_close') return ['item', 'close', a.id, ...flags({ reason: '--reason', handoffs: '--handoff' }), ...auth()];
  if (name === 'item_reopen') return ['item', 'reopen', a.id, ...auth()];
  if (name.startsWith('relation_')) return ['relation', name.slice(9), a.kind, a.source, a.target, ...auth()];
  if (name.startsWith('template_')) {
    const words = ['template', name.slice(9)];
    if (name !== 'template_list') words.push(a.name);
    words.push(...flags({ root: '--root', run_id: '--run' }));
    for (const [key, flag] of [['parameters', '--param'], ['existing', '--existing']]) {
      for (const [k, v] of Object.entries(a[key] ?? {})) words.push(flag, `${k}=${v}`);
    }
    return [...words, ...auth()];
  }
  if (name === 'storage_inspect' || name === 'storage_init') return ['storage', name.slice(8)];
  if (name.startsWith('storage_')) return ['storage', name.slice(8), ...(a.operation_id ? [a.operation_id] : []), ...flags({ expected_store_id: '--expected-store-id', expected_generation: '--expected-generation', executors_stopped: '--executors-stopped', acknowledge_loss: '--acknowledge-loss', all_clients_stopped: '--all-clients-stopped' })];
  if (name === 'claim_acquire' || name === 'claim_next') return ['claim', name.slice(6), ...(a.item ? [a.item] : []), '--actor', a.actor, ...sess(), ...flags({ session_record_id: '--session-record' }), ...scope()];
  if (name === 'claim_inspect') return ['claim', 'inspect', a.claim_id];
  if (name === 'claim_list') return ['claim', 'list', ...flags({ item: '--item', current_only: '--current' })];
  if (name === 'claim_release') return ['claim', 'release', a.claim_id, ...sess(), ...flags({ reason: '--reason' })];
  if (name === 'claim_recover' || name === 'claim_reassign') return ['claim', name.slice(6), a.claim_id, '--actor', a.actor, ...(a.session ? sess() : []), ...flags({ reason: '--reason', executors_stopped: '--executors-stopped' })];
  if (name === 'run_start') return ['run', 'start', a.root, ...flags({ default_workspace_id: '--workspace', output_workspace_id: '--output-workspace' })];
  if (name === 'run_list') return ['run', 'list', ...flags({ include_terminal: '--all' })];
  if (name === 'run_inspect') return ['run', 'inspect', a.run_id];
  if (name === 'run_attach' || name === 'run_detach') return ['run', name.slice(4), a.run_id, ...a.items];
  if (name === 'run_squash') return ['run', 'squash', a.run_id, '--summary', '-'];
  if (name === 'run_discard') return ['run', 'discard', a.run_id, ...flags({ all: '--all', items: '--item' })];
  if (name === 'workspace_register') return ['workspace', 'register', a.path, ...flags({ branch: '--branch', commit: '--commit' })];
  if (name === 'workspace_list') return ['workspace', 'list'];
  if (name === 'workspace_inspect') return ['workspace', 'inspect', a.workspace_id];
  if (name === 'workspace_bind') return ['workspace', 'bind', a.item, a.workspace_id];
  if (name === 'workspace_unbind') return ['workspace', 'unbind', a.item];
  if (name === 'workspace_cleanup_begin') return ['workspace', 'cleanup', 'begin', a.workspace_id, '--item', a.item, '--controller-workspace', a.controller_workspace_id];
  if (name === 'workspace_cleanup_report') return ['workspace', 'cleanup', 'report', a.workspace_id, ...(a.removed ? ['--removed'] : ['--failure', a.failure])];
  if (name === 'workspace_cleanup_cancel') return ['workspace', 'cleanup', 'cancel', a.workspace_id];
  if (name === 'session_set') return ['session', 'set', a.run_id, a.name, '--namespace', a.session.namespace, '--session-id', a.session.id, ...(a.availability ? ['--availability', a.availability.state, '--observed-at', a.availability.observed_at] : [])];
  if (name === 'session_list') return ['session', 'list', a.run_id];
  if (name === 'session_remove') return ['session', 'remove', a.run_id, a.name];
  if (name === 'handoff_create') return ['handoff', 'create', ...flags({ from_items: '--from', to_items: '--to', body: '--body', workspace_id: '--workspace' }), ...(a.session ? sess() : []), ...auth()];
  if (name === 'handoff_inspect') return ['handoff', 'inspect', a.handoff_id];
  if (name === 'handoff_list') return ['handoff', 'list', ...flags({ to_item: '--to' })];
  if (name === 'handoff_receivers') return ['handoff', 'receivers', a.handoff_id, ...flags({ to_items: '--to' })];
  if (name === 'handoff_prune') return ['handoff', 'prune', ...(a.ids ?? [])];
  throw new Error(`unmapped operation ${name}`);
}
function cli(root, name, args) {
  const p = spawnSync(binary, ['--json', '--worktree', root, ...command(name, args), ...(args.bogus ? ['--bogus'] : [])], {
    encoding: 'utf8', timeout: 30000,
    input: name === 'item_repair' ? Buffer.from(args.raw_hex, 'hex') : name === 'run_squash' ? args.summary : undefined,
  });
  assert.equal(p.error, undefined);
  const v = JSON.parse(p.stdout);
  assert.equal(p.status === 0, v.ok === true, p.stderr || p.stdout);
  if (!v.ok) {
    const code = v.error.code;
    const expected = ['invalid_argument', 'usage'].includes(code) ? 2 : ['not_found', 'missing_id', 'ambiguous_id'].includes(code) ? 3 : ['invalid_format', 'unsupported_format', 'coordination_unavailable', 'recovery_required', 'invalid_graph', 'identity_mismatch'].includes(code) ? 4 : ['io', 'permission', 'unsupported_project'].includes(code) ? 1 : 5;
    assert.equal(p.status, expected, JSON.stringify(v));
  }
  return v.ok ? v.result : { error: v.error };
}
function processOutput(root, requests) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, ['mcp'], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '', err = '';
    const timer = setTimeout(() => child.kill('SIGKILL'), 30000);
    child.stdout.setEncoding('utf8').on('data', x => out += x);
    child.stderr.setEncoding('utf8').on('data', x => err += x);
    child.on('error', reject);
    child.on('close', status => {
      clearTimeout(timer);
      try {
        assert.equal(status, 0, err);
        resolve(out.trim().split('\n').map(x => JSON.parse(x)));
      } catch (e) { reject(e); }
    });
    child.stdin.on('error', reject);
    child.stdin.end(requests.map(x => JSON.stringify(x)).join('\n') + '\n');
  });
}
const initialize = { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'beta-verification', version: '1' } } };
async function mcp(root, name, args) {
  const responses = await processOutput(root, [initialize, { jsonrpc: '2.0', method: 'notifications/initialized' }, { jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name, arguments: { ...args, worktree: root } } }]);
  assert.equal(responses[0].result.protocolVersion, '2025-06-18');
  const r = responses.find(x => x.id === 2);
  assert.equal(r.error, undefined, JSON.stringify(r));
  const v = r.result.structuredContent;
  assert.equal(r.result.isError, Boolean(v.error), JSON.stringify(r));
  assert.deepEqual(JSON.parse(r.result.content[0].text), v);
  return v;
}
function snapshot(root) {
  const files = {};
  function walk(dir) {
    if (!fs.existsSync(dir)) return;
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      if (e.name.startsWith('.') || e.name === 'coordination.lock') continue;
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else {
        const source = fs.readFileSync(p, 'utf8');
        // Entity YAML is emitted as JSON; parse it so raw recovery bytes and
        // filesystem fingerprints can be normalized structurally, not erased.
        files[path.relative(root, p)] = source.startsWith('{') ? JSON.parse(source) : source;
      }
    }
  }
  walk(path.join(root, '.work'));
  walk(path.join(root, '.git/work'));
  walk(path.join(root, 'linked/.work'));
  return files;
}
// Preserve every semantic field. Only generated identities, paths, clocks and ID
// ordering differ between equivalent fixtures. Raw hex is decoded before comparison.
function normalizer(root) {
  const ids = new Map();
  function string(s) {
    s = s.split(root).join('<fixture>');
    s = s.replace(/\b[0-9a-f]{40}\b/g, '<fixture-commit>');
    s = s.replace(/w-([0-9a-f]{32}|[0-9a-f]{8})\b/g, (value, prefix) => {
      const id = [...ids.keys()].find(x => x.startsWith(prefix));
      return id ? `w-${ids.get(id)}` : value;
    });
    s = s.replace(/[0-9a-f]{32}/g, id => {
      if (!ids.has(id)) ids.set(id, `<id-${ids.size}>`);
      return ids.get(id);
    });
    return s.replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z/g, '<time>');
  }
  function normalize(v, key = '') {
    if (['dev', 'ino', 'mtime', 'ctime'].includes(key)) return '<filesystem-observation>';
    if (typeof v === 'string') return string(key === 'raw_hex' ? Buffer.from(v, 'hex').toString('utf8') : v);
    if (Array.isArray(v)) return v.map(x => normalize(x)).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
    if (v && typeof v === 'object') {
      if (typeof v.id === 'string') string(v.id); // seed before abbreviated display_id
      return Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b)).map(([k, x]) => [string(k), normalize(x, k)]));
    }
    return v;
  }
  return normalize;
}
function writeTemplate(root, name, source) {
  fs.mkdirSync(path.join(root, '.work/templates'), { recursive: true });
  fs.writeFileSync(path.join(root, `.work/templates/${name}.yaml`), source);
}
async function scenario(mode) {
  const root = fixture(mode);
  if (mode === 'cli') {
    const help = spawnSync(binary, ['--help'], { encoding: 'utf8', timeout: 30000 });
    assert.equal(help.status, 0);
    assert.match(help.stdout, /run start\|inspect\|list\|attach\|detach\|squash\|discard/);
    const contextHelp = spawnSync(binary, ['workspace', '--help'], { encoding: 'utf8', timeout: 30000 });
    assert.equal(contextHelp.status, 0);
    assert.doesNotMatch(contextHelp.stdout, /finalization is not implemented/);
  }
  const normalize = normalizer(root), transcript = [];
  const invoke = mode === 'cli' ? cli : mcp;
  async function call(name, args = {}, code, cwd = root) {
    covered.add(name);
    if (!samples.has(name)) samples.set(name, args);
    const result = await invoke(cwd, name, args);
    if (code) assert.equal(result.error?.code, code, `${name}: ${JSON.stringify(result)}`);
    else assert.equal(result.error, undefined, `${name}: ${JSON.stringify(result)}`);
    // Domain failures retain their complete structured payload for parity.
    transcript.push([name, normalize(result)]);
    return result;
  }
  const item = async title => (await call('item_create', { title, body: text })).item.id;
  await call('discover');
  await call('storage_inspect');
  await call('item_list');
  assert.equal(fs.existsSync(path.join(root, '.git/work')), false);
  await call('storage_init');
  assert.equal((await call('storage_init')).changed, false);
  writeTemplate(root, 'plan', 'format_version: 2\nname: plan\nparameters: [feature]\nitems: [{key: root, title: "Deliver {{feature}}", body: "Plan {{feature}}"}]\n');
  await call('template_list');
  await call('template_validate', { name: 'plan' });
  const before = snapshot(root);
  await call('template_preview', { name: 'plan', parameters: { feature: 'β {{literal}}' } });
  assert.deepEqual(snapshot(root), before);
  await call('template_preview', { name: 'plan', parameters: {} }, 'invalid_argument');
  const planned = await call('template_expand', { name: 'plan', parameters: { feature: 'β {{literal}}' } });
  const rid = planned.items[0].id;
  assert.equal(planned.run_id, null);
  assert.deepEqual(fs.readdirSync(path.join(root, '.git/work/runs')), []);
  const branch = linked(root);
  const local = await call('workspace_register', { path: root });
  const remote = await call('workspace_register', { path: branch, branch: 'fixture-linked', commit: git(branch, 'rev-parse', 'HEAD') });
  const wid = local.workspace.id, bid = remote.workspace.id;
  // Commit SHA is an actual fixture observation, not a product-generated identity.
  transcript.at(-1)[1].workspace.commit = '<fixture-commit>';
  assert.equal((await call('workspace_register', { path: branch })).changed, false);
  // Real divergent, uncommitted whole-file content; never mix fields from main.
  const branchFile = path.join(branch, `.work/items/${rid}.md`);
  const original = fs.readFileSync(branchFile, 'utf8');
  fs.writeFileSync(branchFile, original.replaceAll('β {{literal}}', 'branch body α'));
  await call('workspace_bind', { item: rid, workspace_id: bid });
  assert.equal((await call('item_inspect', { id: rid })).item.body.includes('branch body α'), true);
  assert.equal((await call('item_inspect', { id: rid, view: 'checkout' })).item.body.includes('β {{literal}}'), true);
  await call('workspace_bind', { item: rid, workspace_id: wid });
  await call('workspace_list');
  await call('workspace_inspect', { workspace_id: bid });
  const rootBytes = fs.readFileSync(path.join(root, `.work/items/${rid}.md`));
  const run = (await call('run_start', { root: rid, default_workspace_id: bid, output_workspace_id: wid })).run.id;
  assert.equal((await call('run_start', { root: rid, default_workspace_id: bid, output_workspace_id: wid })).changed, false);
  writeTemplate(branch, 'steps', 'format_version: 2\nname: steps\nparameters: [feature]\ndefaults: {model: external, thinking: high}\nitems:\n  - {key: a, title: "Build {{feature}}", body: "Result {{feature}}", persistence: wisp, labels: [beta], priority: 0}\n  - {key: b, title: "Test {{feature}}", persistence: wisp, labels: [beta], priority: 1}\n  - {key: c, title: "Retain {{feature}}", labels: [beta]}\nedges:\n  - {from: local:c, kind: depends_on, to: local:a}\n  - {from: local:c, kind: depends_on, to: local:b}\n');
  const expanded = await call('template_expand', { name: 'steps', run_id: run, parameters: { feature: 'β' } }, undefined, branch);
  const { a, b, c } = Object.fromEntries(expanded.items.map(x => [x.key, x.id]));
  const wisp = id => path.join(root, `.git/work/runs/${run}/items/${id}.md`);
  assert.equal(fs.existsSync(wisp(a)), true);
  assert.equal(fs.existsSync(wisp(b)), true);
  assert.equal(fs.existsSync(path.join(branch, `.work/items/${c}.md`)), true);
  assert.equal(fs.existsSync(path.join(root, `.work/items/${c}.md`)), false);
  const spare = await item('Independent material');
  await call('run_attach', { run_id: run, items: [spare] });
  await call('run_detach', { run_id: run, items: [spare] });
  await call('workspace_unbind', { item: spare });
  await call('item_update', { id: spare, title: 'Independent updated', labels: ['beta'], priority: 4 });
  await call('relation_add', { kind: 'related', source: spare, target: rid });
  await call('relation_remove', { kind: 'related', source: spare, target: rid });
  await call('item_ready', { run_id: run, labels_all: ['beta'], priority_max: 1, persistence: 'wisp' });
  const named = await call('session_set', { run_id: run, name: 'worker', session, availability: { state: 'unknown', observed_at: '2026-10-01T00:00:00Z' } });
  await call('session_list', { run_id: run });
  const first = (await call('claim_acquire', { item: a, actor: 'worker', session, session_record_id: named.session_record.id })).claim;
  const acquisitionFile = path.join(root, `.git/work/claims/${first.id}.yaml`);
  const acquisitionBytes = fs.readFileSync(acquisitionFile);
  await call('claim_acquire', { item: a, actor: 'worker', session }, 'claim_conflict');
  const pair = claim => ({ claim_id: claim.id, session: claim.session });
  const second = (await call('claim_next', { run_id: run, labels_all: ['beta'], actor: 'other', session })).claim;
  assert.equal(second.item_id, b);
  assert.notEqual(second.id, first.id);
  assert.equal((await call('claim_next', { run_id: run, actor: 'empty', session })).changed, false);
  await call('session_set', { run_id: run, name: 'worker', session: { ...session, id: 'replacement' } });
  await call('item_close', { id: a }, 'claim_conflict');
  await call('claim_list', { current_only: true });
  await call('claim_inspect', { claim_id: first.id });
  const handoff = (await call('handoff_create', { from_items: [a], to_items: [a, c], body: text, authorization: [pair(first)], session, workspace_id: bid })).handoff.id;
  await call('handoff_inspect', { handoff_id: handoff });
  await call('handoff_list', { to_item: a });
  await call('handoff_receivers', { handoff_id: handoff, to_items: [a, c] });
  await call('claim_release', { claim_id: first.id, session, reason: 'continue later' });
  assert.equal((await call('claim_release', { claim_id: first.id, session })).changed, false);
  await call('item_update', { id: a, title: 'stale write', authorization: [pair(first)] }, 'stale_claim');
  const resumed = await call('claim_acquire', { item: a, actor: 'resumed', session });
  assert.equal(resumed.item.incoming_handoffs[0].body, text);
  const replacement = await call('claim_reassign', { claim_id: resumed.claim.id, actor: 'controller', session: { ...session, id: 'next' }, reason: 'stopped fixture owner', executors_stopped: true });
  await call('item_close', { id: a, reason: 'Build passed; retained in handoff', authorization: [pair(replacement.claim)] });
  assert.deepEqual(fs.readFileSync(acquisitionFile), acquisitionBytes, 'ending ownership must not rewrite its acquisition');
  await call('claim_recover', { claim_id: second.id, actor: 'controller', reason: 'stopped fixture executor', executors_stopped: true });
  const newB = (await call('claim_acquire', { item: b, actor: 'restarted', session })).claim;
  await call('item_close', { id: b, reason: 'Tests passed; retained in handoff', authorization: [pair(newB)], handoffs: [{ from_items: [b], to_items: [c], body: text }] });
  assert.equal((await call('item_inspect', { id: c })).item.incoming_handoffs.length, 2);
  await call('item_close', { id: c, reason: 'Results retained for digest' });
  await call('handoff_prune');
  await call('session_remove', { run_id: run, name: 'worker' });
  await call('session_set', { run_id: run, name: 'retained-until-cleanup', session });
  const finished = await call('run_inspect', { run_id: run });
  assert.equal(finished.finished, true);
  assert.equal(fs.existsSync(wisp(a)), true); // finishing alone retains files
  assert.equal(fs.existsSync(wisp(b)), true);
  // Retained material dependencies must be explicitly removed after recording
  // their results; completion does not make dangling references safe.
  await call('run_squash', { run_id: run, summary: text }, 'reference_blocked');
  await call('relation_remove', { kind: 'depends_on', source: c, target: a });
  await call('relation_remove', { kind: 'depends_on', source: c, target: b });
  const materialBytes = fs.readFileSync(path.join(branch, `.work/items/${c}.md`));
  const digest = await call('run_squash', { run_id: run, summary: text });
  assert.equal(digest.phase, 'finalized');
  assert.equal((await call('run_squash', { run_id: run, summary: text })).changed, false);
  await call('run_squash', { run_id: run, summary: 'different' }, 'run_conflict');
  assert.deepEqual(fs.readFileSync(path.join(root, `.work/items/${rid}.md`)), rootBytes);
  assert.deepEqual(fs.readFileSync(path.join(branch, `.work/items/${c}.md`)), materialBytes);
  const digestFile = path.join(root, `.work/digests/${rid}/${run}.md`);
  assert.equal(fs.readFileSync(digestFile, 'utf8').endsWith(text), true);
  const inspected = await call('item_inspect', { id: rid });
  assert.equal(inspected.item.state, 'open');
  assert.equal(inspected.item.digests[0].body, text);
  assert.equal(fs.existsSync(wisp(a)), false);
  assert.deepEqual(fs.readdirSync(path.join(root, `.git/work/runs/${run}/sessions`)).filter(x => !x.startsWith('.')), []);
  await call('run_list', { include_terminal: true });
  // A fresh run, selected abandonment, outside-reference refusal, and full
  // disposal retain material progress and independent missing-receiver context.
  const next = (await call('run_start', { root: rid })).run.id;
  assert.notEqual(next, run);
  writeTemplate(root, 'discard', 'format_version: 2\nname: discard\nitems: [{key: x, title: Abandon, persistence: wisp}, {key: y, title: Keep, persistence: wisp}]\n');
  const disposable = await call('template_expand', { name: 'discard', run_id: next });
  const [x, y] = disposable.items.map(v => v.id);
  await call('run_discard', { run_id: next, items: [rid] }, 'invalid_argument');
  await call('relation_add', { kind: 'depends_on', source: spare, target: x });
  await call('run_discard', { run_id: next, items: [x] }, 'reference_blocked');
  await call('relation_remove', { kind: 'depends_on', source: spare, target: x });
  const orphan = (await call('handoff_create', { from_items: [x], to_items: [x], body: text })).handoff.id;
  await call('run_discard', { run_id: next, items: [x] });
  assert.equal(fs.existsSync(path.join(root, `.git/work/runs/${next}/items/${y}.md`)), true);
  const retained = await call('handoff_prune', { ids: [orphan] });
  assert.equal(retained.retained.length, 1);
  await call('run_discard', { run_id: next, all: true });
  assert.equal((await call('run_discard', { run_id: next, all: true })).changed, false);
  assert.equal(fs.existsSync(path.join(root, `.git/work/handoffs/${orphan}.md`)), true);
  assert.equal(fs.existsSync(digestFile), true);
  // Explicit cleanup: transfer completed material authority before begin;
  // exercise failure/cancel/retry then remove only our own linked fixture.
  git(branch, 'add', '.work');
  git(branch, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'retain fixture results');
  fs.copyFileSync(path.join(branch, `.work/items/${c}.md`), path.join(root, `.work/items/${c}.md`));
  await call('workspace_bind', { item: c, workspace_id: wid });
  const cleanup = await item('Cleanup fixture');
  await call('workspace_cleanup_begin', { workspace_id: bid, item: cleanup, controller_workspace_id: wid });
  await call('workspace_bind', { item: spare, workspace_id: bid }, 'workspace_busy');
  await call('workspace_cleanup_report', { workspace_id: bid, removed: false, failure: text });
  await call('workspace_cleanup_cancel', { workspace_id: bid });
  await call('workspace_cleanup_begin', { workspace_id: bid, item: cleanup, controller_workspace_id: wid });
  git(root, 'worktree', 'remove', branch);
  await call('workspace_cleanup_report', { workspace_id: bid, removed: true });
  await call('workspace_inspect', { workspace_id: bid }, 'not_found');
  assert.equal((await call('item_inspect', { id: cleanup })).item.state, 'open');
  await call('item_close', { id: spare });
  await call('item_reopen', { id: spare });
  // Diagnosed invalid source, byte inspection and real adapter repair.
  const file = path.join(root, `.work/items/${spare}.md`), valid = fs.readFileSync(file);
  fs.writeFileSync(file, 'invalid source\n');
  await call('item_diagnose');
  await call('item_inspect_raw', { id: spare });
  await call('item_repair', { id: spare, raw_hex: valid.toString('hex') });
  assert.deepEqual(fs.readFileSync(file), valid);
  // Real client restarts above never change ownership. Explicit generation
  // recovery is separate from claim recovery and never reconstructs claims.
  const owner = (await call('claim_acquire', { item: spare, actor: 'before-reset', session })).claim;
  const metadata = (await call('storage_inspect')).storage.metadata;
  const lock = path.join(root, '.git/work/coordination.lock'), inode = fs.statSync(lock).ino;
  const orphanBytes = fs.readFileSync(path.join(root, `.git/work/handoffs/${orphan}.md`));
  fs.renameSync(path.join(root, '.git/work/handoffs'), path.join(root, '.git/work/recovery/fixture-handoffs')); // simulated directory loss without erasing bytes
  const damage = await call('storage_inspect');
  assert.equal(damage.storage.coordination_available, false);
  await call('claim_acquire', { item: cleanup, actor: 'refused', session }, 'recovery_required');
  const reset = await call('storage_recreate', { expected_store_id: metadata.store_id, expected_generation: metadata.recovery_generation, executors_stopped: true, acknowledge_loss: true });
  assert.equal(fs.statSync(lock).ino, inode);
  assert.notEqual(reset.storage.metadata.recovery_generation, metadata.recovery_generation);
  await call('storage_recover', { operation_id: reset.operation_id, executors_stopped: true, acknowledge_loss: true });
  await call('item_update', { id: spare, title: 'former generation', authorization: [pair(owner)] }, 'stale_claim');
  assert.equal((await call('claim_list', { current_only: true })).claims.length, 0);
  assert.equal(fs.existsSync(digestFile), true);
  assert.deepEqual(fs.readFileSync(path.join(root, `.git/work/recovery/fixture-handoffs/${orphan}.md`)), orphanBytes);
  transcript.push(['files', normalize(snapshot(root))]);
  // Every delivered beta adapter must reject unknown inputs before touching
  // storage, even when its sampled run/workspace is no longer current.
  const invalidBefore = snapshot(root);
  for (const name of covered) {
    if (!/^(claim_|run_|workspace_|session_|handoff_|storage_|template_)/.test(name)) continue;
    const invalid = ['run_attach', 'run_detach'].includes(name)
      ? { ...samples.get(name), items: [] }
      : { ...samples.get(name), bogus: true };
    const result = await invoke(root, name, invalid);
    // CLI syntax errors use legacy usage (exit 2); MCP schema errors use
    // invalid_argument. Both mean rejected input, not a domain mutation.
    assert.ok(['invalid_argument', 'usage'].includes(result.error?.code), `${name} invalid input: ${JSON.stringify(result)}`);
    transcript.push([`${name}:invalid`, 'rejected_input']);
  }
  assert.deepEqual(snapshot(root), invalidBefore, 'invalid adapter requests must not publish');
  return transcript;
}
async function races() {
  const root = fixture('races');
  cli(root, 'storage_init', {});
  const parent = cli(root, 'item_create', { title: 'Parallel execution root' }).item.id;
  const run = cli(root, 'run_start', { root: parent }).run.id;
  writeTemplate(root, 'race', 'format_version: 2\nname: race\nparameters: [feature]\nitems: [{key: a, title: "Contended {{feature}}", persistence: wisp}, {key: b, title: "Parallel {{feature}}", persistence: wisp}, {key: c, title: "Parallel {{feature}}", persistence: wisp}]\nedges: [{from: local:a, kind: parent, to: root}, {from: local:b, kind: parent, to: root}, {from: local:c, kind: parent, to: root}]\n');
  const children = cli(root, 'template_expand', { name: 'race', run_id: run, parameters: { feature: 'β' } }).items;
  const id = children.find(x => x.key === 'a').id;
  const branch = linked(root);
  // Start real CLI and MCP clients concurrently. No notification channel exists.
  const acquire = { item: id, actor: 'racer', session };
  const p = spawn(binary, ['--json', '--worktree', root, ...command('claim_acquire', acquire)], { stdio: ['ignore', 'pipe', 'pipe'] });
  const output = new Promise((resolve, reject) => {
    let stdout = '', stderr = '';
    const timer = setTimeout(() => p.kill('SIGKILL'), 30000);
    p.stdout.on('data', x => stdout += x);
    p.stderr.on('data', x => stderr += x);
    p.on('error', reject);
    p.on('close', status => { clearTimeout(timer); try { const v = JSON.parse(stdout); assert.equal(status === 0, v.ok, stderr); resolve(v.ok ? v.result : { error: v.error }); } catch (e) { reject(e); } });
  });
  const result = await Promise.all([output, mcp(branch, 'claim_acquire', acquire)]);
  assert.equal(result.filter(v => v.claim).length, 1);
  assert.ok(['claim_conflict', 'storage_busy'].includes(result.find(v => v.error).error.code));
  assert.equal((await mcp(branch, 'claim_acquire', acquire)).error.code, 'claim_conflict');
  const current = await mcp(root, 'claim_list', { current_only: true });
  assert.equal(current.claims.length, 1);
  assert.equal(current.claims[0].claim.id, result.find(v => v.claim).claim.id);
  // Repeat on multiple eligible items, with atomic claim-next across views.
  const requests = [{ actor: 'one', session, root: parent, run_id: run }, { actor: 'two', session, root: parent, run_id: run }];
  const roots = [root, branch];
  const claims = await Promise.all(roots.map((cwd, i) => mcp(cwd, 'claim_next', requests[i])));
  // Nonblocking lock contention is explicit, not a successful empty scope.
  for (let i = 0; i < claims.length; i++) if (claims[i].error?.code === 'storage_busy') claims[i] = await mcp(roots[i], 'claim_next', requests[i]);
  assert.ok(claims.every(v => v.claim), JSON.stringify(claims));
  assert.notEqual(claims[0].claim.item_id, claims[1].claim.item_id);
  assert.ok(claims.every(v => v.claim.run_id === run));
  // Connected race winners finish their own work, then retain one digest.
  for (const claim of [result.find(v => v.claim).claim, ...claims.map(v => v.claim)]) {
    const closed = await mcp(root, 'item_close', { id: claim.item_id, reason: 'raced child passed', authorization: [{ claim_id: claim.id, session: claim.session }] });
    assert.equal(closed.error, undefined, JSON.stringify(closed));
  }
  assert.equal((await mcp(root, 'run_inspect', { run_id: run })).finished, true);
  assert.equal((await mcp(root, 'run_squash', { run_id: run, summary: text })).phase, 'finalized');
  assert.equal((await mcp(root, 'item_inspect', { id: parent })).item.state, 'open');
}
try {
  const list = await processOutput(fixture('inventory'), [initialize, { jsonrpc: '2.0', id: 2, method: 'tools/list' }]);
  const names = list.find(x => x.id === 2).result.tools.map(t => t.name).sort();
  const a = await scenario('cli');
  const b = await scenario('mcp');
  for (let i = 0; i < a.length; i++) assert.deepEqual(b[i], a[i], `CLI/MCP mismatch at step ${i} (${a[i][0]})`);
  assert.equal(b.length, a.length);
  assert.deepEqual([...covered].sort(), names, 'every advertised capability must be exercised');
  await races();
  console.log(`beta CLI/MCP acceptance passed (${names.length} operations, linked fixtures, races, restart, retention and cleanup): ${binary}`);
} finally {
  fs.rmSync(scratch, { recursive: true, force: true });
}
