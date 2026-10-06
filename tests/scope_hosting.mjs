// Real public hosting regressions; shared fixture helpers do not mock Work.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { binary, record, execute, git, cli, success, fail, errorFields, write, digest, fingerprint, Fixture, MCP, host, itemPath, declaration, copyScope } from './support/public.mjs';

// Selected reference regressions, adapted for this fresh bounded surface.

test('inherited depth, siblings, whole uncommitted sources, authoritative mutations and physical refusal', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('A'), child = f.create('child', a), leaf = f.create('leaf', child), sibling = f.create('sibling');
    success(f.primary, 'relation', 'add', 'depends_on', leaf, sibling);
    const original = fs.readFileSync(itemPath(f.primary, leaf)), unrelated = fs.readFileSync(itemPath(f.primary, sibling));
    assert.equal(fs.existsSync(path.join(f.primary, '.work/scopes')), false); f.commit(); const linked = f.linked('child-host');
    const moved = host(f.primary, a, linked); assert.equal(moved.changed, true); assert.equal(moved.scope.host, linked);
    const raw = Buffer.concat([original, Buffer.from('\nUNCOMMITTED authoritative body')]); fs.writeFileSync(itemPath(linked, leaf), raw);
    const fromPrimary = success(f.primary, 'item', 'inspect', leaf).item; assert.equal(fromPrimary.context.source_worktree, linked); assert.ok(fromPrimary.body.endsWith('UNCOMMITTED authoritative body')); assert.equal(fromPrimary.context.scope_id, a); assert.equal(fromPrimary.relations.depends_on[0], sibling);
    assert.deepEqual(fs.readFileSync(itemPath(f.primary, leaf)), original); assert.deepEqual(fs.readFileSync(itemPath(f.primary, sibling)), unrelated);
    assert.equal(success(linked, 'scope', 'inspect', leaf).scope.host, linked);
    const physical = success(f.primary, 'item', 'inspect', leaf, '--view', 'physical'); assert.equal(physical.authoritative, false); assert.equal(physical.item.body.endsWith('UNCOMMITTED authoritative body'), false); assert.equal(physical.item.context.source_worktree, f.primary); assert.equal(physical.execution.state, 'unavailable'); assert.equal(physical.item.executable, null);
    assert.equal(success(linked, 'scope', 'inspect', 'root', '--view', 'physical').scope.host, f.primary);
    success(f.primary, 'item', 'update', leaf, '--title', 'CLI routed'); assert.equal(success(linked, 'item', 'inspect', leaf, '--view', 'physical').item.title, 'CLI routed');
    m = new MCP(f.primary); await m.init(); await m.good('item_update', { id: leaf, title: 'MCP routed' }); assert.equal((await m.good('item_inspect', { id: leaf })).item.title, 'MCP routed');
    const before = fingerprint(f.root); fail(f.primary, 'invalid_argument', 'item', 'update', leaf, '--title', 'forbidden', '--view', 'physical'); await m.bad('item_update', { id: leaf, title: 'forbidden', view: 'physical' }, 'invalid_argument'); await m.bad('scope_host', { scope: a, destination: linked, expected_inventory: '0'.repeat(64), view: 'physical' }, 'invalid_argument'); assert.deepEqual(fingerprint(f.root), before);
    const current = success(f.primary, 'scope', 'inspect', a).scope.inventory_sha256; assert.equal(success(f.primary, 'scope', 'host', a, '--destination', linked, '--expected-inventory', current).changed, false);
    assert.deepEqual(fs.readFileSync(itemPath(f.primary, sibling)), unrelated); assert.deepEqual(fs.readFileSync(itemPath(f.primary, leaf)), original); assert.equal(success(f.primary, 'item', 'inspect', a).item.parent, null);
    assert.equal(fs.existsSync(path.join(linked, '.gitignore')), false); assert.deepEqual(JSON.parse(fs.readFileSync(path.join(linked, '.work/scopes', a, 'scope.yaml'))), { format_version: 2, scope_id: a, execution: { mode: 'inherit' } });
  } finally { if (m) await m.close(); f.close(); }
});


test('hosting an implicit ancestor moves existing parent-local child references, not child hosts or material bytes', async () => {
  const f = new Fixture(); try {
    const a = f.create('implicit A'), b = f.create('boundary B', a), c = f.create('descendant', b); f.commit(); const ownB = f.linked('own-B'), ownA = f.linked('own-A'); host(f.primary, b, ownB);
    const before = fs.readFileSync(itemPath(ownB, c)); host(f.primary, a, ownA);
    assert.equal(success(f.primary, 'scope', 'inspect', b).scope.parent_scope_id, a); assert.equal(success(f.primary, 'scope', 'inspect', b).scope.host, ownB); assert.equal(success(f.primary, 'scope', 'inspect', a).scope.host, ownA);
    assert.equal(fs.existsSync(path.join(f.primary, '.work/scopes/root/children', `${b}.yaml`)), false); assert.equal(fs.existsSync(path.join(ownA, '.work/scopes', a, 'children', `${b}.yaml`)), true); assert.deepEqual(fs.readFileSync(itemPath(ownB, c)), before);
  } finally { f.close(); }
});


test('invalid input/candidate on a fresh repository has no bootstrap/progress writes; obsolete tools are not aliases', async () => {
  const f = new Fixture(); let m; try {
    const before = fingerprint(f.root); fail(f.primary, 'invalid_argument', 'scope', 'host', 'root', '--destination', f.primary, '--expected-inventory', '0'.repeat(64)); fail(f.primary, 'invalid_argument', 'item', 'create', '--title', 'x', '--parent', 'bad-id'); fail(f.primary, 'invalid_argument', 'item', 'create', '--title', 'x', '--view', 'physical');
    m = new MCP(f.primary); await m.init(); await m.bad('scope_host', { scope: 'root', destination: f.primary, expected_inventory: null }, 'invalid_argument'); await m.bad('item_create', { title: 'x', unknown: true }, 'invalid_argument'); const old = await m.request('tools/call', { name: 'workspace_register', arguments: {} }); assert.equal(old.error.code, -32602); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});

function yamlJSON(value) { const sorted = v => Array.isArray(v) ? v.map(sorted) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map(k => [k, sorted(v[k])])) : v; return `${JSON.stringify(sorted(value), null, 2)}\n`; }
const epoch = '2025-01-01T00:00:00.000Z';
function front(header, body) { return `---\r\n${JSON.stringify(header)}\r\n---\r\n${body}`; }

function runPath(cwd, scope, run, ...parts) { return path.join(cwd, '.work/scopes', scope, 'runs', run, ...parts); }
function prepareRun(cwd, scope, run) { write(path.join(cwd, '.work/scopes', scope, 'scope.yaml'), { format_version: 2, scope_id: scope, execution: { mode: 'boundary', run_id: run } }); write(runPath(cwd, scope, run, 'run.yaml'), { format_version: 2, id: run, created_at: epoch, phase: 'active' }); }
function prepareNamedClaim(cwd, scope, run, item, session, claim, name) {
  const identity = { namespace: 'fixture', id: `identity-${session}` };
  write(runPath(cwd, scope, run, 'sessions', `${session}.yaml`), { format_version: 2, id: session, name, actor: 'Alice', identity, registered_at: epoch, state: 'active' });
  write(runPath(cwd, scope, run, 'claims', `${claim}.yaml`), { format_version: 2, id: claim, item_id: item, actor: 'Alice', identity, acquired_at: epoch, named_session: { run_id: run, session_record_id: session } });
}


test('ordinary material reparenting maintains several stored references without changing hosts or opaque sources', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('implicit A'), b = f.create('B', a), c = f.create('C', a), p = f.create('new parent'); f.commit(); const ownB = f.linked('B'), ownC = f.linked('C'); host(f.primary, b, ownB); host(f.primary, c, ownC); host(f.primary, p, f.primary);
    const beforeA = fs.readFileSync(itemPath(f.primary, a)), bBytes = fs.readFileSync(itemPath(ownB, b)), cBytes = fs.readFileSync(itemPath(ownC, c)); m = new MCP(f.primary); await m.init(); const changed = await m.good('item_update', { id: a, parent: p }); assert.equal(changed.partial.tree_changes[0].step, 3); assert.ok(changed.item.recovery_path); assert.deepEqual(fs.readFileSync(changed.item.recovery_path), beforeA);
    for (const [item, home, bytes] of [[b, ownB, bBytes], [c, ownC, cBytes]]) { const read = await m.good('scope_inspect', { scope: item }); assert.equal(read.scope.parent_scope_id, p); assert.equal(read.scope.host, home); assert.deepEqual(fs.readFileSync(itemPath(home, item)), bytes); assert.equal(fs.existsSync(path.join(f.primary, '.work/scopes/root/children', `${item}.yaml`)), false); assert.equal(fs.existsSync(path.join(f.primary, '.work/scopes', p, 'children', `${item}.yaml`)), true); }
    assert.equal((await m.good('scope_inspect', { scope: 'root' })).execution.pending.length, 0);
  } finally { if (m) await m.close(); f.close(); }
});


test('prepared interrupted hosting resumes exact next publication, reconciles a stale counter, and refuses newly discovered files', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('A'), unrelated = f.create('unrelated continuation guard'); f.commit(); const destination = f.linked('destination'), intruder = f.linked('additional candidate'); const inventory = success(f.primary, 'scope', 'inspect', a).scope.inventory_sha256;
    const marks = [{ path: `.work/items/${a}.md`, sha256: digest(fs.readFileSync(itemPath(f.primary, a))) }]; const op = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa'; const rootAnchor = { format_version: 2, scope_id: 'root', execution: { mode: 'uninitialized' } }, childAnchor = { format_version: 2, scope_id: a, execution: { mode: 'inherit' } };
    const progress = path.join(success(f.primary, 'discover').git_common_dir, 'work-beta2/relocations', `${op}.yaml`);
    const plan = { format_version: 2, id: op, created_at: epoch, scope_id: a, mode: 'host', source_hosts: [f.primary], destination, inventory: { scope_ids: [a], files: marks, sha256: inventory }, declarations: [{ checkout: destination, before: { exists: false }, before_scope_ids: [], after: { format_version: 2, scope_ids: [a] } }], new_boundaries: [{ scope_id: 'root', checkout: f.primary, anchor_before: { exists: false }, anchor_after: rootAnchor }, { scope_id: a, checkout: destination, anchor_before: { exists: false }, anchor_after: childAnchor, parent_scope_id: 'root', child_before: { exists: false } }], tree_references: [], executors_quiescent: false, step: 1 };
    // Use the implementation's YAML serialization (JSON is a YAML 1.2 subset).
    write(progress, plan); write(path.join(f.primary, '.work/scopes/root/scope.yaml'), yamlJSON(rootAnchor)); write(path.join(destination, '.work/scopes', a, 'scope.yaml'), yamlJSON(childAnchor));
    m = new MCP(f.primary); await m.init(); const pending = await m.good('scope_inspect', { scope: a }); assert.equal(pending.execution.state, 'pending');
    const foreign = path.join(f.root, 'foreign-publication-root'); fs.mkdirSync(foreign); const foreignPlan = structuredClone(plan); foreignPlan.new_boundaries[0].checkout = foreign; write(progress, foreignPlan); const foreignBefore = fingerprint(f.root); await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op }, 'source_unavailable'); assert.deepEqual(fingerprint(f.root), foreignBefore); assert.equal(fs.existsSync(path.join(foreign, '.work')), false); const wrongIdentity = structuredClone(plan); wrongIdentity.new_boundaries[1].anchor_after.scope_id = unrelated; write(progress, wrongIdentity); const wrongBefore = fingerprint(f.root); await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), wrongBefore); write(progress, plan);
    const extra = runPath(destination, a, 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', 'run.yaml'); write(extra, { format_version: 2, id: 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', created_at: epoch, phase: 'abandoned', ended_at: epoch }); const before = fingerprint(f.root); await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op }, 'conflict'); assert.deepEqual(fingerprint(f.root), before); fs.rmSync(path.dirname(extra), { recursive: true });
    const unrelatedBytes = fs.readFileSync(itemPath(f.primary, unrelated)); fs.writeFileSync(itemPath(f.primary, unrelated), unrelatedBytes.toString().replace('priority: 2', 'priority: 2\ndepends_on: ["cccccccccccc4ccc8ccccccccccccccc"]')); const invalidGraph = fingerprint(f.root); await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), invalidGraph); fs.writeFileSync(itemPath(f.primary, unrelated), unrelatedBytes);
    const newDeclaration = declaration(intruder); write(newDeclaration, { format_version: 2, scope_ids: [a] }); fs.appendFileSync(itemPath(intruder, a), '\nnew divergent declared candidate'); const candidateAdded = fingerprint(f.root); await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op }, 'conflict'); assert.deepEqual(fingerprint(f.root), candidateAdded); assert.equal(fs.existsSync(progress), true); fs.unlinkSync(newDeclaration);
    const resumed = await m.good('scope_host', { scope: a, destination: destination + '/../destination', expected_inventory: inventory, continue_id: op }); assert.equal(resumed.scope.host, destination); assert.equal(resumed.execution.pending.length, 0); assert.equal(fs.existsSync(progress), false);
    const body = fs.readFileSync(itemPath(destination, a)); const fresh = resumed.scope.inventory_sha256; assert.equal((await m.good('scope_host', { scope: a, destination, expected_inventory: fresh })).changed, false); assert.deepEqual(fs.readFileSync(itemPath(destination, a)), body);
  } finally { if (m) await m.close(); f.close(); }
});


test('foreign-home wisp ancestry routes material children without relocating wisps or blocking unrelated creates', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('material boundary'), sibling = f.create('unrelated');
    const run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', wisp = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb';
    prepareRun(f.primary, 'root', run);
    const wispFile = runPath(f.primary, 'root', run, 'items', `${wisp}.md`);
    write(wispFile, front({ format_version: 1, id: wisp, title: 'foreign-home ancestor', completion: 'manual', state: 'open', priority: 2, parent: a }, '\nretained wisp body\nlast'));
    const material = f.create('material below wisp', wisp); f.commit(); const linked = f.linked('host');
    const original = fs.readFileSync(itemPath(f.primary, material)), temporary = fs.readFileSync(wispFile);
    host(f.primary, a, linked); fs.appendFileSync(itemPath(linked, material), '\nuncommitted foreign-home ancestry');
    assert.equal(success(f.primary, 'item', 'inspect', material).item.context.source_worktree, linked);
    m = new MCP(f.primary); await m.init(); const read = await m.good('item_inspect', { id: material });
    assert.ok(read.item.body.endsWith('uncommitted foreign-home ancestry'));
    await m.good('item_update', { id: material, title: 'routed material' });
    await m.good('item_create', { title: 'unrelated child', parent: sibling });
    assert.deepEqual(fs.readFileSync(itemPath(f.primary, material)), original); assert.deepEqual(fs.readFileSync(wispFile), temporary);
    const physical = await m.good('item_inspect', { id: wisp, view: 'physical' });
    assert.equal(physical.item.context.owner_run_id, run); assert.equal(physical.authoritative, false); assert.equal(physical.execution.state, 'unavailable');
    assert.equal((await m.good('item_list', { view: 'physical' })).items.some(i => i.id === wisp), false);
    assert.ok((await m.good('item_list', { view: 'physical', include_terminal: true })).items.some(i => i.id === wisp));
    assert.equal((await m.good('item_inspect', { id: wisp })).item.context.source_worktree, f.primary);
  } finally { if (m) await m.close(); f.close(); }
});


test('orphaned authoritative run ownership and unidentified material corruption remain diagnostic and refuse writes', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('orphan root'), leaf = f.create('claimed leaf', a);
    const run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', session = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', acquisition = 'cccccccccccc4ccc8ccccccccccccccc';
    prepareRun(f.primary, a, run); prepareNamedClaim(f.primary, a, run, leaf, session, acquisition, 'owner');
    m = new MCP(f.primary); await m.init();
    const inspect = await m.good('item_inspect', { id: leaf });
    assert.equal(inspect.execution.state, 'unavailable'); assert.ok(inspect.diagnostics.some(d => d.message.includes('orphaned')));
    const before = fingerprint(f.root); await m.bad('item_update', { id: leaf, title: 'must not bypass orphan ownership' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
    // Restore the explicit stored tree externally; no lifecycle commands are claimed.
    write(path.join(f.primary, '.work/scopes/root/scope.yaml'), { format_version: 2, scope_id: 'root', execution: { mode: 'uninitialized' } });
    write(path.join(f.primary, '.work/scopes/root/children', `${a}.yaml`), { format_version: 2, child_scope_id: a });
    assert.equal((await m.good('item_inspect', { id: leaf })).item.current_claim.claim.id, acquisition);
    write(path.join(f.primary, '.work/items/notes.md'), 'unidentified malformed bytes');
    const diagnostic = await m.good('item_diagnose'); assert.ok(diagnostic.diagnostics.some(d => d.path.endsWith('/notes.md')));
    const damaged = fingerprint(f.root); await m.bad('item_ready', {}, 'invalid_source'); await m.bad('item_update', { id: leaf, title: 'blocked', session: 'owner', claim_id: acquisition }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), damaged);
  } finally { if (m) await m.close(); f.close(); }
});


test('a stored boundary root reparents using its projected header and preserves source bytes and hosting', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('boundary root'), p = f.create('new boundary parent'), leaf = f.create('leaf', a);
    host(f.primary, a, f.primary); host(f.primary, p, f.primary); const bytes = fs.readFileSync(itemPath(f.primary, leaf));
    m = new MCP(f.primary); await m.init(); const changed = await m.good('item_update', { id: a, parent: p });
    assert.equal(changed.partial.tree_changes[0].step, 3); assert.equal((await m.good('scope_inspect', { scope: a })).scope.parent_scope_id, p);
    assert.equal((await m.good('scope_inspect', { scope: a })).scope.host, f.primary); assert.deepEqual(fs.readFileSync(itemPath(f.primary, leaf)), bytes);
    assert.equal(fs.existsSync(path.join(f.primary, '.work/scopes/root/children', `${a}.yaml`)), false);
    assert.equal(fs.existsSync(path.join(f.primary, '.work/scopes', p, 'children', `${a}.yaml`)), true);
  } finally { if (m) await m.close(); f.close(); }
});



test('inherited boundary host changes are refused before publishing reparent progress or source bytes', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('inherited run boundary'), p = f.create('dedicated remote parent');
    prepareRun(f.primary, a, 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa');
    write(path.join(f.primary, '.work/scopes/root/scope.yaml'), { format_version: 2, scope_id: 'root', execution: { mode: 'uninitialized' } });
    write(path.join(f.primary, '.work/scopes/root/children', `${a}.yaml`), { format_version: 2, child_scope_id: a });
    f.commit(); const linked = f.linked('remote'); host(f.primary, p, linked);
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root);
    await m.bad('item_update', { id: a, parent: p }, 'conflict');
    fail(f.primary, 'conflict', 'item', 'update', a, '--parent', p);
    assert.deepEqual(fingerprint(f.root), before); assert.equal(success(f.primary, 'scope', 'inspect', a).scope.host, f.primary);
  } finally { if (m) await m.close(); f.close(); }
});


test('an unclaimed active wisp prevents ancestor reparenting into another run before any writes', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('ancestor'), p = f.create('other run boundary');
    const run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', other = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', wisp = 'cccccccccccc4ccc8ccccccccccccccc';
    prepareRun(f.primary, 'root', run); host(f.primary, p, f.primary); prepareRun(f.primary, p, other);
    write(runPath(f.primary, 'root', run, 'items', `${wisp}.md`), front({ format_version: 1, id: wisp, title: 'active unclaimed', completion: 'manual', state: 'open', priority: 2, parent: a }, 'opaque wisp'));
    m = new MCP(f.primary); await m.init(); assert.equal((await m.good('item_inspect', { id: wisp })).item.current_claim, null);
    const before = fingerprint(f.root); await m.bad('item_update', { id: a, parent: p }, 'wisp_run_mismatch'); assert.deepEqual(fingerprint(f.root), before);
    // Raw noncooperating damage is also reported rather than presented as healthy.
    const raw = fs.readFileSync(itemPath(f.primary, a), 'utf8'); fs.writeFileSync(itemPath(f.primary, a), raw.replace('priority: 2', `priority: 2\nparent: "${p}"`));
    assert.ok((await m.good('item_diagnose')).diagnostics.some(d => d.code === 'wisp_run_mismatch'));
  } finally { if (m) await m.close(); f.close(); }
});


test('explicit readiness run filters reject unknown and terminal runs; empty malformed directories stay diagnostic', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('terminal boundary'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa'; host(f.primary, a, f.primary); prepareRun(f.primary, a, run);
    write(runPath(f.primary, a, run, 'run.yaml'), { format_version: 2, id: run, created_at: epoch, phase: 'finalized', ended_at: epoch });
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root);
    await m.bad('item_ready', { run_id: run }, 'run_not_current'); await m.bad('item_ready', { run_id: 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb' }, 'not_found'); assert.deepEqual(fingerprint(f.root), before);
    assert.ok((await m.good('item_list', { run_id: run, include_terminal: true })).items.some(i => i.id === a));
    fs.mkdirSync(path.join(f.primary, '.work/scopes/not-a-scope/runs/not-a-run'), { recursive: true });
    const diagnostic = await m.good('item_diagnose'); assert.ok(diagnostic.diagnostics.some(d => d.message.includes('directory layout')));
    const damaged = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'blocked layout' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), damaged);
  } finally { if (m) await m.close(); f.close(); }
});


test('malformed host declarations never return stale primary bodies as authoritative', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('malformed authority'), leaf = f.create('leaf', a); f.commit(); const linked = f.linked('host'); host(f.primary, a, linked); fs.appendFileSync(itemPath(linked, leaf), '\nnew authoritative bytes');
    write(declaration(linked), 'format_version: 2\nscope_ids: []\nscope_ids: []\n');
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root);
    await m.bad('item_inspect', { id: leaf }, 'invalid_source'); fail(f.primary, 'invalid_source', 'item', 'inspect', leaf); assert.deepEqual(fingerprint(f.root), before);
    assert.equal((await m.good('scope_inspect', { scope: a })).scope.host, null);
    const actual = await m.good('item_inspect', { id: leaf, worktree: linked, view: 'physical' }); assert.equal(actual.authoritative, false); assert.ok(actual.item.body.endsWith('new authoritative bytes'));
  } finally { if (m) await m.close(); f.close(); }
});


test('conflicting current acquisitions expose unresolved ownership rather than a chosen actor', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('conflicting ownership'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa';
    prepareRun(f.primary, 'root', run);
    prepareNamedClaim(f.primary, 'root', run, a, 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', 'cccccccccccc4ccc8ccccccccccccccc', 'one');
    prepareNamedClaim(f.primary, 'root', run, a, 'dddddddddddd4ddd8ddddddddddddddd', 'eeeeeeeeeeee4eee8eeeeeeeeeeeeeee', 'two');
    m = new MCP(f.primary); await m.init(); const inspected = await m.good('item_inspect', { id: a }); assert.equal(inspected.item.current_claim, null); assert.equal(inspected.item.executable, null); assert.ok(inspected.diagnostics.some(d => d.code === 'claim_conflict' && d.item_id === a));
    const before = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'cannot choose owner', session: 'one' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('an inventoried host rebound to a foreign repository never grants authority or accepts writes', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('repository identity'), leaf = f.create('protected leaf', a); f.commit(); const linked = f.linked('host'); host(f.primary, a, linked);
    fs.unlinkSync(path.join(linked, '.git')); git(linked, 'init', '--initial-branch=foreign');
    m = new MCP(f.primary); await m.init(); const scope = await m.good('scope_inspect', { scope: a });

    assert.ok(scope.diagnostics.some(d => d.code === 'source_unavailable')); assert.ok(scope.scope.candidates.some(c => c.checkout === linked && !c.available));
    const before = fingerprint(f.root); const denied = await m.call('item_update', { id: leaf, title: 'never overwrite foreign repository' }); assert.equal(denied.isError, true); assert.deepEqual(fingerprint(f.root), before);
    const listed = await m.good('item_list'); assert.ok(listed.diagnostics.length); assert.equal(listed.items.some(i => i.id === leaf), false);
  } finally { if (m) await m.close(); f.close(); }
});


test('an acquisition must reside in its wisp owner run, independently of named identity qualification', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('owning run'), b = f.create('other run'); host(f.primary, a, f.primary); host(f.primary, b, f.primary);
    const first = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', second = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', wisp = 'cccccccccccc4ccc8ccccccccccccccc'; prepareRun(f.primary, a, first); prepareRun(f.primary, b, second);
    write(runPath(f.primary, b, second, 'items', `${wisp}.md`), front({ format_version: 1, id: wisp, title: 'run owned', completion: 'manual', state: 'open', priority: 2, parent: b }, 'owned bytes'));
    prepareNamedClaim(f.primary, a, first, wisp, 'dddddddddddd4ddd8ddddddddddddddd', 'eeeeeeeeeeee4eee8eeeeeeeeeeeeeee', 'wronghome');
    m = new MCP(f.primary); await m.init(); const inspected = await m.good('item_inspect', { id: wisp }); assert.equal(inspected.item.context.owner_run_id, second); assert.equal(inspected.item.current_claim, null); assert.ok(inspected.diagnostics.some(d => d.code === 'claim_conflict'));
    const before = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'malformed ownership cannot be ignored' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('a frozen tree edit refuses a newly affected stored boundary without losing recovery', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('moving ancestor'), b = f.create('original child', a), c = f.create('new child', a), p = f.create('new stored parent'); host(f.primary, b, f.primary); host(f.primary, p, f.primary);
    const oldItem = fs.readFileSync(itemPath(f.primary, a)), rootReference = path.join(f.primary, '.work/scopes/root/children', `${b}.yaml`), newReference = path.join(f.primary, '.work/scopes', p, 'children', `${b}.yaml`), oldReference = fs.readFileSync(rootReference);
    m = new MCP(f.primary); await m.init(); await m.good('item_update', { id: a, parent: p }); const afterItem = fs.readFileSync(itemPath(f.primary, a));
    fs.writeFileSync(itemPath(f.primary, a), oldItem); fs.unlinkSync(newReference); fs.writeFileSync(rootReference, oldReference);
    const run = '11111111111141118111111111111111', session = '22222222222242228222222222222222', claim = '33333333333343338333333333333333'; prepareRun(f.primary, 'root', run); prepareNamedClaim(f.primary, 'root', run, a, session, claim, 'owner');
    const op = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', anchor = path.join(f.primary, '.work/scopes/root/scope.yaml'), progress = path.join(f.primary, '.work/scopes/root/progress/start', `${op}.yaml`);
    const plan = { format_version: 2, id: op, created_at: epoch, scope_id: 'root', anchor_before: { exists: true, sha256: digest(fs.readFileSync(anchor)) }, tree_change: { item_id: a, operation: 'item_update', parent_before: null, parent_after: p, old_parent_scope_id: 'root', new_parent_scope_id: p, item_before: digest(oldItem), item_after: digest(afterItem), step: 0 }, tree_references: [{ parent_scope_id: p, child_scope_id: b, before: { exists: false }, after: { format_version: 2, child_scope_id: b } }, { parent_scope_id: 'root', child_scope_id: b, before: { exists: true, sha256: digest(oldReference) } }], step: 0 };
    write(progress, plan); const newAnchor = path.join(f.primary, '.work/scopes', c, 'scope.yaml'), newChildReference = path.join(f.primary, '.work/scopes/root/children', `${c}.yaml`); write(newAnchor, { format_version: 2, scope_id: c, execution: { mode: 'inherit' } }); write(newChildReference, { format_version: 2, child_scope_id: c });
    assert.equal((await m.good('item_inspect', { id: a })).item.current_claim, null); const before = fingerprint(f.root); await m.bad('item_update', { id: a, parent: p, continue_id: op }, 'claim_conflict'); assert.deepEqual(fingerprint(f.root), before); await m.bad('item_update', { id: a, parent: p, continue_id: op, session: 'owner' }, 'conflict'); assert.deepEqual(fingerprint(f.root), before); assert.equal(fs.existsSync(progress), true);
    fs.rmSync(path.dirname(newAnchor), { recursive: true }); fs.unlinkSync(newChildReference); const resumed = await m.good('item_update', { id: a, parent: p, continue_id: op, session: 'owner' }); assert.equal(resumed.partial.tree_changes[0].step, 3); assert.equal(fs.existsSync(progress), false); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); assert.equal((await m.good('item_inspect', { id: a })).item.current_claim.claim.id, claim); await m.bad('item_update', { id: a, title: 'owner still required' }, 'claim_conflict');
  } finally { if (m) await m.close(); f.close(); }
});


test('inactive scope-copy corruption stays physical and does not poison authoritative ownership', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('hosted records'), leaf = f.create('healthy leaf', a); f.commit(); const linked = f.linked('host'); host(f.primary, a, linked);
    const run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', handoff = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb'; prepareRun(linked, a, run);
    fs.cpSync(path.join(linked, '.work/scopes', a), path.join(f.primary, '.work/scopes', a), { recursive: true });
    const stale = runPath(f.primary, a, run, 'handoffs', `${handoff}.md`); write(stale, '---\nnot: [valid\n---\nstale inactive bytes');
    const staleDigest = path.join(f.primary, '.work/digests', a, `${run}.md`); write(staleDigest, 'broken inactive digest');
    m = new MCP(f.primary); await m.init(); const resolved = await m.good('item_inspect', { id: leaf }); assert.equal(resolved.diagnostics.length, 0); assert.deepEqual(resolved.item.incoming_handoffs, []); assert.notEqual(resolved.item.executable, null);
    assert.ok((await m.good('item_diagnose', { view: 'physical' })).diagnostics.some(d => d.path === stale));
    await m.good('item_update', { id: leaf, title: 'healthy authority remains editable' }); assert.equal(fs.readFileSync(staleDigest, 'utf8'), 'broken inactive digest');
  } finally { if (m) await m.close(); f.close(); }
});


test('duplicate terminal wisps retain both whole physical sources and expose unresolved ownership', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('terminal owners'), first = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', second = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', wisp = 'cccccccccccc4ccc8ccccccccccccccc'; host(f.primary, a, f.primary);
    for (const [run, body] of [[first, 'first body'], [second, 'second body']]) { write(runPath(f.primary, a, run, 'run.yaml'), { format_version: 2, id: run, created_at: epoch, phase: 'abandoned', ended_at: epoch }); write(runPath(f.primary, a, run, 'items', `${wisp}.md`), front({ format_version: 1, id: wisp, title: body, completion: 'manual', state: 'open', priority: 2, parent: a }, body)); }
    m = new MCP(f.primary); await m.init(); const diagnosed = await m.good('item_diagnose'); assert.ok(diagnosed.diagnostics.some(d => d.code === 'claim_conflict'));
    const conflicting = await m.call('item_inspect', { id: wisp }); assert.equal(conflicting.isError, true);
    const before = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'never ignore duplicate identities' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('ended barriers cannot select retained active manifests for material or hosting mutations', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('ended barrier'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa'; host(f.primary, a, f.primary); prepareRun(f.primary, a, run);
    write(path.join(f.primary, '.work/scopes', a, 'scope.yaml'), { format_version: 2, scope_id: a, execution: { mode: 'ended', run_id: run } });
    m = new MCP(f.primary); await m.init(); const scope = await m.good('scope_inspect', { scope: a }); assert.ok(scope.diagnostics.some(d => d.message === 'unselected active run'));
    const before = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'reject unselected execution' }, 'invalid_source'); await m.bad('scope_host', { scope: a, destination: f.primary, expected_inventory: scope.scope.inventory_sha256 }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('terminal-wisp membership is independent of unavailable execution and unrelated diagnostics', async () => {
  const f = new Fixture(); let m; try {

    const a = f.create('history'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', wisp = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb'; host(f.primary, a, f.primary);
    write(runPath(f.primary, a, run, 'run.yaml'), { format_version: 2, id: run, created_at: epoch, phase: 'abandoned', ended_at: epoch }); write(runPath(f.primary, a, run, 'items', `${wisp}.md`), front({ format_version: 1, id: wisp, title: 'terminal', completion: 'manual', state: 'open', priority: 2, parent: a }, 'retained bytes'));
    m = new MCP(f.primary); await m.init(); assert.equal((await m.good('item_list')).items.some(i => i.id === wisp), false);
    write(path.join(f.primary, '.work/items/notes.md'), 'malformed unrelated source'); const list = await m.good('item_list'); assert.ok(list.diagnostics.length); assert.equal(list.items.some(i => i.id === wisp), false);
    const all = await m.good('item_list', { include_terminal: true }); assert.ok(all.items.some(i => i.id === wisp && i.context.execution_state === 'unavailable'));
  } finally { if (m) await m.close(); f.close(); }
});


test('duplicate acquisition identities in different active run homes never grant current ownership', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('owner A'), b = f.create('owner B'), first = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', second = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', claim = 'cccccccccccc4ccc8ccccccccccccccc', one = 'dddddddddddd4ddd8ddddddddddddddd', two = 'eeeeeeeeeeee4eee8eeeeeeeeeeeeeee'; host(f.primary, a, f.primary); host(f.primary, b, f.primary); prepareRun(f.primary, a, first); prepareRun(f.primary, b, second); prepareNamedClaim(f.primary, a, first, a, one, claim, 'first'); prepareNamedClaim(f.primary, b, second, b, two, claim, 'second');
    m = new MCP(f.primary); await m.init(); for (const id of [a, b]) { const inspected = await m.good('item_inspect', { id }); assert.equal(inspected.item.current_claim, null); assert.ok(inspected.diagnostics.some(d => d.message === 'duplicate authoritative acquisition ID')); }
    const before = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'duplicate cannot authorize', identity: { namespace: 'fixture', id: `identity-${one}` }, claim_id: claim }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('partial parent-removal retries require the original operation and exact edge; missing sources never terminate MCP', async () => {
  const f = new Fixture(); let m; try {
    const p = f.create('old stored parent'); host(f.primary, p, f.primary); const a = f.create('moving ancestor', p), b = f.create('stored child', a); host(f.primary, b, f.primary);
    const oldA = fs.readFileSync(itemPath(f.primary, a)), previous = path.join(f.primary, '.work/scopes', p, 'children', `${b}.yaml`), previousBytes = fs.readFileSync(previous);
    m = new MCP(f.primary); await m.init(); const removed = await m.good('relation_remove', { source: a, kind: 'parent', target: p }); const newA = fs.readFileSync(itemPath(f.primary, a)); assert.deepEqual(fs.readFileSync(removed.item.recovery_path), oldA); write(previous, previousBytes);
    const op = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', progress = path.join(f.primary, '.work/scopes', p, 'progress/start', `${op}.yaml`), anchor = path.join(f.primary, '.work/scopes', p, 'scope.yaml');
    write(progress, { format_version: 2, id: op, created_at: epoch, scope_id: p, anchor_before: { exists: true, sha256: digest(fs.readFileSync(anchor)) }, tree_change: { item_id: a, operation: 'relation_remove', parent_before: p, parent_after: null, old_parent_scope_id: p, new_parent_scope_id: 'root', item_before: digest(oldA), item_after: digest(newA), step: 2 }, tree_references: [{ parent_scope_id: 'root', child_scope_id: b, before: { exists: false }, after: { format_version: 2, child_scope_id: b } }, { parent_scope_id: p, child_scope_id: b, before: { exists: true, sha256: digest(previousBytes) } }], step: 2 });
    const before = fingerprint(f.root); await m.bad('relation_remove', { source: a, kind: 'parent', target: b, continue_id: op }, 'conflict'); await m.bad('item_update', { id: a, clear_parent: true, continue_id: op }, 'conflict'); await m.bad('relation_remove', { source: a, kind: 'related', target: b, continue_id: op }, 'conflict'); assert.deepEqual(fingerprint(f.root), before);
    fail(f.primary, 'conflict', 'relation', 'remove', 'parent', a, b, '--continue', op); assert.deepEqual(fingerprint(f.root), before);
    const completed = await m.good('relation_remove', { source: a, kind: 'parent', target: p, continue_id: op }); assert.equal(completed.partial.tree_changes[0].step, 3); assert.equal(fs.existsSync(progress), false);
    const oldB = fs.readFileSync(itemPath(f.primary, b)); await m.good('relation_remove', { source: b, kind: 'parent', target: a }); const newB = fs.readFileSync(itemPath(f.primary, b)), ownerAnchor = path.join(f.primary, '.work/scopes', b, 'scope.yaml'), brokenProgress = path.join(f.primary, '.work/scopes', b, 'progress/start', `${op}.yaml`);
    write(brokenProgress, { format_version: 2, id: op, created_at: epoch, scope_id: b, anchor_before: { exists: true, sha256: digest(fs.readFileSync(ownerAnchor)) }, tree_change: { item_id: b, operation: 'relation_remove', parent_before: a, parent_after: null, old_parent_scope_id: 'root', new_parent_scope_id: 'root', item_before: digest(oldB), item_after: digest(newB), step: 1 }, tree_references: [], step: 1 }); fs.unlinkSync(itemPath(f.primary, b));
    const damaged = fingerprint(f.root); await m.bad('relation_remove', { source: b, kind: 'related', target: a, continue_id: op }, 'conflict'); assert.deepEqual(fingerprint(f.root), damaged); assert.ok((await m.good('item_diagnose')).diagnostics.length);
  } finally { if (m) await m.close(); f.close(); }
});


test('raw material inspection preserves header-ID and interrupted filename-repair source selection', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('original raw'), original = itemPath(f.primary, a), misnamed = path.join(f.primary, '.work/items/misnamed.md'), bytes = fs.readFileSync(original); fs.renameSync(original, misnamed);
    m = new MCP(f.primary); await m.init(); for (const view of ['resolved', 'physical']) { const result = await m.good('item_inspect_raw', { id: a, view }); assert.equal(result.source.path, misnamed); assert.equal(result.source.raw_hex, bytes.toString('hex')); assert.ok(result.source.diagnostics.length); const cliRaw = success(f.primary, 'item', 'inspect', a, '--raw', '--view', view); assert.deepEqual(cliRaw.source, result.source); }
    const repaired = Buffer.from(bytes.toString().replace('original raw', 'repaired canonical')); assert.notDeepEqual(repaired, bytes); write(original, repaired); const before = fingerprint(f.root);
    const interrupted = await m.good('item_inspect_raw', { id: a }); assert.equal(interrupted.source.path, misnamed); assert.equal(interrupted.source.raw_hex, bytes.toString('hex')); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('malformed retained wisps stay available for whole-byte raw inspection in both views', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('retained namespace'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', wisp = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb'; host(f.primary, a, f.primary); prepareRun(f.primary, a, run);
    const file = runPath(f.primary, a, run, 'items', `${wisp}.md`), bytes = Buffer.from(front({ format_version: 1, id: wisp, completion: 'manual', state: 'open', priority: 2, parent: a }, '\nopaque malformed wisp α\nlast')); write(file, bytes);
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root); for (const view of ['resolved', 'physical']) { const read = await m.good('item_inspect_raw', { id: wisp, view }); assert.equal(read.source.path, file); assert.equal(read.source.raw_hex, bytes.toString('hex')); assert.ok(read.source.diagnostics.length); assert.equal(success(f.primary, 'item', 'inspect', wisp, '--raw', '--view', view).source.raw_hex, bytes.toString('hex')); } assert.deepEqual(fingerprint(f.root), before);
    await m.bad('item_update', { id: a, title: 'invalid execution cannot authorize' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('declared but unreachable scopes expose candidates without root-owned stale bodies', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('unreachable boundary'), leaf = f.create('stale leaf', a); f.commit(); const linked = f.linked('declared'); host(f.primary, a, linked); fs.appendFileSync(itemPath(linked, leaf), '\nnew authoritative bytes'); fs.unlinkSync(path.join(f.primary, '.work/scopes/root/children', `${a}.yaml`));
    m = new MCP(f.primary); await m.init(); const scope = await m.good('scope_inspect', { scope: a }); assert.equal(scope.scope.host, null); assert.ok(scope.scope.candidates.some(c => c.declared && c.checkout === linked)); assert.ok(scope.diagnostics.some(d => d.message === 'declared scope is unreachable through parent-local references'));
    const before = fingerprint(f.root); const refused = await m.call('item_inspect', { id: leaf }); assert.equal(refused.isError, true); const raw = await m.call('item_inspect_raw', { id: leaf }); assert.equal(raw.isError, true); assert.deepEqual(fingerprint(f.root), before);
    const physical = await m.good('item_inspect', { id: leaf, view: 'physical', worktree: linked }); assert.ok(physical.item.body.endsWith('new authoritative bytes')); assert.equal(physical.authoritative, false); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('frozen declaration membership preserves unrelated destination authority during boundary establishment', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('new boundary'), b = f.create('existing destination co-host');
    f.commit(); const destination = f.linked('destination'); host(f.primary, b, destination);
    fs.appendFileSync(itemPath(destination, b), '\nuncommitted unrelated destination body');
    const unrelated = fs.readFileSync(itemPath(destination, b));
    const inventory = success(f.primary, 'scope', 'inspect', a).scope.inventory_sha256;
    const original = fs.readFileSync(declaration(destination)), op = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa';
    const anchor = { format_version: 2, scope_id: a, execution: { mode: 'inherit' } };
    const progress = path.join(success(f.primary, 'discover').git_common_dir, 'work-beta2/relocations', `${op}.yaml`);
    const plan = { format_version: 2, id: op, created_at: epoch, scope_id: a, mode: 'host', source_hosts: [f.primary], destination,
      inventory: { scope_ids: [a], files: [{ path: `.work/items/${a}.md`, sha256: digest(fs.readFileSync(itemPath(f.primary, a))) }], sha256: inventory },
      declarations: [{ checkout: destination, before: { exists: true, sha256: digest(original) }, before_scope_ids: [b], after: { format_version: 2, scope_ids: [a,b].sort() } }],
      new_boundaries: [{ scope_id: a, checkout: destination, anchor_before: { exists: false }, anchor_after: anchor, parent_scope_id: 'root', child_before: { exists: false } }],
      tree_references: [], executors_quiescent: false, step: 2 };
    write(path.join(destination, '.work/scopes', a, 'scope.yaml'), yamlJSON(anchor));
    write(path.join(f.primary, '.work/scopes/root/children', `${a}.yaml`), yamlJSON({ format_version: 2, child_scope_id: a }));
    m = new MCP(f.primary); await m.init();
    for (const step of [2,3]) {
      const altered = structuredClone(plan); altered.step = step; altered.declarations[0].after.scope_ids = [a];
      if(step===3) write(declaration(destination), yamlJSON(altered.declarations[0].after));
      write(progress, altered); const before = fingerprint(f.root);
      await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op }, 'invalid_source');
      assert.deepEqual(fingerprint(f.root), before); assert.equal(fs.existsSync(progress), true);
    }
    write(declaration(destination), original); write(progress, plan);
    const resumed = await m.good('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: op });
    assert.equal(resumed.scope.host, destination); assert.equal((await m.good('scope_inspect', { scope: b })).scope.host, destination);
    assert.deepEqual(fs.readFileSync(itemPath(destination, b)), unrelated);
    assert.deepEqual(JSON.parse(fs.readFileSync(declaration(destination))).scope_ids, [a,b].sort());
    assert.equal(fs.existsSync(progress), false);
  } finally { if(m) await m.close(); f.close(); }
});


test('malformed ending and unreadable competing acquisition leave ownership unresolved', async () => {
  const f = new Fixture(); let m, hidden; try {
    const a = f.create('ownership namespace'), leaf = f.create('claimed', a), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', session = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', claim = 'cccccccccccc4ccc8ccccccccccccccc', other = 'dddddddddddd4ddd8ddddddddddddddd'; host(f.primary, a, f.primary); prepareRun(f.primary, a, run); prepareNamedClaim(f.primary, a, run, leaf, session, claim, 'owner');
    m = new MCP(f.primary); await m.init(); assert.equal((await m.good('item_inspect', { id: leaf })).item.current_claim.current, true); const ending = runPath(f.primary, a, run, 'claims', `${claim}.end.yaml`); write(ending, { format_version: 2, claim_id: claim }); const malformed = await m.good('item_inspect', { id: leaf }); assert.equal(malformed.item.current_claim, null); assert.equal(malformed.item.executable, null); assert.ok(malformed.diagnostics.length); fs.unlinkSync(ending);
    hidden = runPath(f.primary, a, run, 'claims', `${other}.yaml`); write(hidden, { format_version: 2, id: other, item_id: leaf, actor: 'Bob', identity: { namespace: 'fixture', id: 'unreadable other' }, acquired_at: epoch }); fs.chmodSync(hidden, 0); const unreadable = await m.good('item_inspect', { id: leaf }); assert.equal(unreadable.item.current_claim, null); assert.equal(unreadable.item.executable, null); assert.ok(unreadable.diagnostics.length); fs.chmodSync(hidden, 0o644); fs.unlinkSync(hidden); hidden = null; assert.equal((await m.good('item_inspect', { id: leaf })).item.current_claim.current, true);
  } finally { if (hidden) fs.chmodSync(hidden, 0o644); if (m) await m.close(); f.close(); }
});


test('full and unique-prefix inspection preserve host ambiguity and candidate evidence in both transports', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('ambiguous namespace'), leaf = f.create('ambiguous descendant', a); f.commit(); const one = f.linked('one'), two = f.linked('two'); host(f.primary, a, one); copyScope(one, two, a); write(declaration(two), { format_version: 2, scope_ids: [a] }); m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root);
    for (const id of [a, 'w-' + a.slice(0, 8), leaf, 'w-' + leaf.slice(0, 8)]) { const error = await m.bad('item_inspect', { id }, 'host_ambiguous'); assert.ok(error.diagnostics.some(d => d.code === 'host_ambiguous')); assert.ok(error.candidates.some(c => c.declared && c.checkout === one)); assert.ok(error.candidates.some(c => c.declared && c.checkout === two)); const cliError = fail(f.primary, 'host_ambiguous', 'item', 'inspect', id); assert.ok(cliError.candidates.length >= 2); }
    for (const id of [a, leaf]) await m.bad('item_inspect_raw', { id }, 'host_ambiguous'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('unavailable relative-path linked hosts retain metadata declarations without stale fallback', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('relative hosting root'), leaf = f.create('stale primary', a); f.commit(); const linked = path.join(f.root, 'relative'); git(f.primary, 'worktree', 'add', '--relative-paths', '--detach', linked); host(f.primary, a, linked); copyScope(linked, f.primary, a); fs.appendFileSync(itemPath(linked, leaf), '\nuncommitted relative-host body'); const metadata = success(linked, 'discover').git_dir; assert.equal(path.isAbsolute(fs.readFileSync(path.join(metadata, 'gitdir'), 'utf8').trim()), false); fs.renameSync(linked, path.join(f.root, 'unavailable-owned-copy'));
    m = new MCP(f.primary); await m.init(); const scope = await m.good('scope_inspect', { scope: a }); assert.equal(scope.scope.host, linked); assert.ok(scope.scope.candidates.some(c => c.declared && c.checkout === linked && !c.available)); const before = fingerprint(f.root); await m.bad('item_inspect', { id: leaf }, 'source_unavailable'); const mutated = await m.call('item_update', { id: leaf, title: 'not the missing host' }); assert.equal(mutated.isError, true); errorFields(mutated.structuredContent.error); assert.deepEqual(fingerprint(f.root), before); assert.ok(!fs.readFileSync(itemPath(f.primary, leaf), 'utf8').endsWith('uncommitted relative-host body'));
  } finally { if (m) await m.close(); f.close(); }
});


test('inactive-only material identities do not create required-source loss or block primary workflows', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('primary live'); f.commit(); const linked = f.linked('inactive-branch'), other = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa'; write(itemPath(linked, other), front({ format_version: 1, id: other, title: 'inactive only', completion: 'manual', state: 'open', priority: 2 }, '\nbranch-only body'));
    m = new MCP(f.primary); await m.init(); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); const list = await m.good('item_list'); assert.deepEqual(list.items.map(i => i.id), [a]); assert.equal(list.items[0].executable, true); await m.good('item_update', { id: a, title: 'primary still writable' }); await m.good('item_create', { title: 'unrelated new primary item' }); const physical = await m.good('item_inspect', { id: other, view: 'physical', worktree: linked }); assert.equal(physical.item.title, 'inactive only'); assert.equal(physical.authoritative, false); assert.equal((await m.good('scope_inspect', { scope: other })).scope.host, null);
  } finally { if (m) await m.close(); f.close(); }
});


test('competing inherited checkout paths withhold subtree authority but keep candidate copies inspectable', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('stored A'), b = f.create('stored B'), c = f.create('inherited C', a), leaf = f.create('leaf', c); f.commit(); const one = f.linked('one'), two = f.linked('two'); host(f.primary, a, one); host(f.primary, b, two); host(f.primary, c, one); write(declaration(one), { format_version: 2, scope_ids: [a] }); copyScope(one, two, c); write(path.join(two, '.work/scopes', b, 'children', `${c}.yaml`), { format_version: 2, child_scope_id: c }); fs.appendFileSync(itemPath(two, leaf), '\ncompeting second-copy bytes');
    m = new MCP(f.primary); await m.init(); const scope = await m.good('scope_inspect', { scope: c }); assert.equal(scope.scope.host, null); assert.ok(scope.diagnostics.some(d => d.code === 'host_ambiguous' && d.message === 'competing inherited hosting paths')); const before = fingerprint(f.root); await m.bad('item_inspect', { id: c }, 'host_ambiguous'); await m.bad('item_inspect', { id: leaf }, 'host_ambiguous'); for (const worktree of [one, two]) { const physical = await m.good('item_inspect_raw', { id: leaf, worktree, view: 'physical' }); assert.equal(physical.source.raw_hex, fs.readFileSync(itemPath(worktree, leaf)).toString('hex')); assert.equal(physical.authoritative, false); } assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('malformed hosted descendant remains diagnostic and its whole raw source stays inspectable', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('hosted namespace'), leaf = f.create('malformed leaf', a), sibling = f.create('healthy sibling', a); f.commit(); const linked = f.linked('host'); host(f.primary, a, linked); const source = itemPath(linked, leaf), original = fs.readFileSync(source), stale = fs.readFileSync(itemPath(f.primary, leaf)), malformed = Buffer.from(original.toString().replace(`parent: "${a}"`, 'parent: [invalid]')); assert.notDeepEqual(malformed, original); write(source, malformed);
    m = new MCP(f.primary); await m.init(); const diagnose = await m.good('item_diagnose'); assert.ok(diagnose.diagnostics.some(d => d.path === source)); assert.ok(!(await m.good('item_list')).items.some(i => i.id === leaf)); const raw = await m.good('item_inspect_raw', { id: leaf }); assert.equal(raw.source.path, source); assert.equal(raw.source.raw_hex, malformed.toString('hex')); const before = fingerprint(f.root); await m.bad('item_update', { id: sibling, title: 'must not ignore invalid source' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before); assert.deepEqual(fs.readFileSync(itemPath(f.primary, leaf)), stale); write(source, original); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0);
  } finally { if (m) await m.close(); f.close(); }
});


test('inactive copied execution does not imply missing coordination in material-only primary bootstrap', async () => {
  const f = new Fixture(); let m; try {
    const a = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', run = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb'; write(itemPath(f.primary, a), front({ format_version: 1, id: a, title: 'uninitialized material', completion: 'manual', state: 'open', priority: 2 })); f.commit(); const linked = f.linked('inactive'); prepareRun(linked, 'root', run); const common = success(f.primary, 'discover').git_common_dir; assert.equal(fs.existsSync(path.join(common, 'work-beta2/coordination.lock')), false);
    m = new MCP(f.primary); await m.init(); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); assert.equal((await m.good('item_inspect', { id: a })).item.context.execution_state, 'uninitialized'); await m.good('item_update', { id: a, title: 'bootstrap remains writable' }); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); assert.equal((await m.good('item_inspect', { id: a })).item.context.execution_state, 'uninitialized');
  } finally { if (m) await m.close(); f.close(); }
});


test('malformed local bounded progress remains visible as unsupported and continuation never reports absence', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('progress subject'), p = f.create('parent'), op = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', file = path.join(f.primary, '.work/scopes/root/progress/start', `${op}.yaml`); write(file, { format_version: 2, id: op, broken: true });
    m = new MCP(f.primary); await m.init(); const inspect = await m.good('scope_inspect', { scope: 'root' }); assert.deepEqual(inspect.execution.pending.find(x => x.path === file), { id: null, kind: 'start', path: file, step: null, supported: false }); assert.ok(inspect.diagnostics.some(d => d.path === file)); const before = fingerprint(f.root); await m.bad('item_update', { id: a, parent: p, continue_id: op }, 'invalid_format'); assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('unresolved malformed descendant copies cannot make resolved raw inspection authoritative', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('hosted parent'), leaf = f.create('unresolved copies', a); f.commit(); const linked = f.linked('host'); host(f.primary, a, linked); for (const worktree of [f.primary, linked]) { const file = itemPath(worktree, leaf), original = fs.readFileSync(file, 'utf8'), malformed = original.replace(`parent: "${a}"`, 'parent: [invalid]'); assert.notEqual(malformed, original); write(file, malformed + '\ncopy-local body ' + worktree); }
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root); const error = await m.bad('item_inspect_raw', { id: leaf }, 'source_unavailable'); assert.ok(error.candidates.some(c => c.checkout === f.primary)); assert.ok(error.candidates.some(c => c.checkout === linked)); fail(f.primary, 'source_unavailable', 'item', 'inspect', leaf, '--raw'); for (const worktree of [f.primary, linked]) { const raw = await m.good('item_inspect_raw', { id: leaf, view: 'physical', worktree }); assert.equal(raw.authoritative, false); assert.equal(raw.source.raw_hex, fs.readFileSync(itemPath(worktree, leaf)).toString('hex')); } assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});


test('unambiguous single-host material preserves ordinary and raw inspection of invalid parent graphs', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('cycle A'), b = f.create('cycle B'), unknown = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa'; m = new MCP(f.primary); await m.init(); for (const parent of [b, unknown]) { for (const [id, target] of [[a, parent], [b, a]]) { const file = itemPath(f.primary, id), header = front({ format_version: 1, id, title: 'invalid hierarchy fixture', completion: 'manual', state: 'open', priority: 2, parent: target }, '\ninspectable opaque body'); write(file, header); }
      const diagnosed = await m.good('item_diagnose'); assert.ok(diagnosed.diagnostics.some(d => d.code === 'invalid_source' && (parent === b ? d.message.startsWith('hierarchy cycle') : d.message.includes(unknown))), JSON.stringify(diagnosed)); assert.ok(!diagnosed.diagnostics.some(d => d.code === 'source_unavailable')); for (const id of [a, b]) { assert.equal((await m.good('item_inspect', { id })).item.body, '\ninspectable opaque body'); const raw = await m.good('item_inspect_raw', { id }); assert.equal(raw.source.raw_hex, fs.readFileSync(itemPath(f.primary, id)).toString('hex')); success(f.primary, 'item', 'inspect', id, '--raw'); } const before = fingerprint(f.root); await m.bad('item_ready', {}, 'invalid_source'); await m.bad('item_update', { id: a, title: 'blocked hierarchy' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before);
    }
  } finally { if (m) await m.close(); f.close(); }
});


test('terminal owning runs cannot authorize current cross-run named claims in active or retiring state', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('named owner root'), b = f.create('claim home'), leaf = f.create('owned material', b), runA = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', runB = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb', session = 'cccccccccccc4ccc8ccccccccccccccc', claim = 'dddddddddddd4ddd8ddddddddddddddd'; host(f.primary, a, f.primary); host(f.primary, b, f.primary); prepareRun(f.primary, a, runA); prepareRun(f.primary, b, runB); prepareNamedClaim(f.primary, a, runA, a, session, 'eeeeeeeeeeee4eee8eeeeeeeeeeeeeee', 'owner'); fs.unlinkSync(runPath(f.primary, a, runA, 'claims', 'eeeeeeeeeeee4eee8eeeeeeeeeeeeeee.yaml')); write(runPath(f.primary, b, runB, 'claims', `${claim}.yaml`), { format_version: 2, id: claim, item_id: leaf, actor: 'Alice', identity: { namespace: 'fixture', id: `identity-${session}` }, acquired_at: epoch, named_session: { run_id: runA, session_record_id: session } });
    m = new MCP(f.primary); await m.init(); await m.good('item_update', { id: leaf, title: 'valid cross-run owner', session: `${runA}/owner` }); write(runPath(f.primary, a, runA, 'run.yaml'), { format_version: 2, id: runA, created_at: epoch, phase: 'abandoned' }); write(path.join(f.primary, '.work/scopes', a, 'scope.yaml'), { format_version: 2, scope_id: a, execution: { mode: 'ended', run_id: runA } }); for (const state of ['active', 'retiring']) { const file = runPath(f.primary, a, runA, 'sessions', `${session}.yaml`), named = JSON.parse(fs.readFileSync(file, 'utf8')); named.state = state; write(file, named); const diagnosed = await m.good('item_diagnose'); assert.ok(diagnosed.diagnostics.some(d => d.code === 'claim_conflict')); assert.equal((await m.good('item_inspect', { id: leaf })).item.current_claim, null); const before = fingerprint(f.root); await m.bad('item_update', { id: leaf, title: 'cannot use ended owner', session: `${runA}/owner` }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before); }
  } finally { if (m) await m.close(); f.close(); }
});


test('valid virtual-root digests never require a root material document or block hosting and material workflows', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('root digest material'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', file = path.join(f.primary, '.work/digests/root', `${run}.md`), raw = Buffer.from(front({ format_version: 2, root_scope_id: 'root', run_id: run }, '\nopaque root digest')); write(file, raw); f.commit(); const linked = f.linked('host'); m = new MCP(f.primary); await m.init(); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); assert.ok((await m.good('item_ready')).items.some(i => i.id === a)); await m.good('item_update', { id: a, title: 'root digest remains valid' }); fs.copyFileSync(itemPath(f.primary, a), itemPath(linked, a)); const scope = await m.good('scope_inspect', { scope: a }); await m.good('scope_host', { scope: a, destination: linked, expected_inventory: scope.scope.inventory_sha256 }); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); assert.deepEqual(fs.readFileSync(file), raw); assert.equal(fs.existsSync(itemPath(f.primary, 'root')), false);
  } finally { if (m) await m.close(); f.close(); }
});


test('inferred authoritative malformed orphan content stays diagnostic while inactive copies never activate', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('implicit material root'), run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', claim = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb'; f.commit(); const linked = f.linked('inactive'); for (const worktree of [f.primary, linked]) { write(path.join(worktree, '.work/scopes', a, 'scope.yaml'), { format_version: 2, scope_id: a, execution: { mode: 'broken' } }); write(runPath(worktree, a, run, 'run.yaml'), { format_version: 2, id: run, phase: 'broken' }); write(runPath(worktree, a, run, 'claims', `${claim}.yaml`), { format_version: 2, id: claim, broken: true }); }
    m = new MCP(f.primary); await m.init(); const diagnosed = await m.good('item_diagnose'); for (const file of [path.join(f.primary, '.work/scopes', a, 'scope.yaml'), runPath(f.primary, a, run, 'run.yaml'), runPath(f.primary, a, run, 'claims', `${claim}.yaml`)]) assert.ok(diagnosed.diagnostics.some(d => d.path === file)); assert.ok(!diagnosed.diagnostics.some(d => d.path?.startsWith(linked))); assert.equal((await m.good('scope_inspect', { scope: a })).scope.stored, false); const before = fingerprint(f.root); await m.bad('item_update', { id: a, title: 'cannot ignore orphan corruption' }, 'invalid_source'); assert.deepEqual(fingerprint(f.root), before); fs.rmSync(path.join(f.primary, '.work/scopes', a), { recursive: true }); assert.equal((await m.good('item_diagnose')).diagnostics.length, 0); await m.good('item_update', { id: a, title: 'inactive corruption remains inactive' });
  } finally { if (m) await m.close(); f.close(); }
});

test('fresh ordinary primary routing follows Git facts across branches, detached HEAD and linked .git files without writes', async () => {
  const f = new Fixture(); let m; try {
    const linked=f.linked('linked');
    for(const args of [['checkout','-b','feature'],['checkout','--detach'],['checkout','main']]) {
      git(f.primary,...args); const before=fingerprint(f.root);
      for(const selected of [f.primary,linked]) {
        const d=success(selected,'discover'); assert.equal(d.primary_checkout,f.primary); assert.equal(d.worktree_root,selected);
        const root=success(selected,'scope','inspect','root'); assert.equal(root.scope.host,f.primary); assert.equal(root.scope.stored,false);
        assert.equal(root.execution.state,'uninitialized');
        for(let restart=0;restart<2;restart++) {
          m=new MCP(selected); await m.init();
          assert.deepEqual(await m.good('discover'),d);
          assert.deepEqual(await m.good('scope_inspect',{scope:'root'}),root);
          const tools=(await m.request('tools/list')).result.tools;
          for(const unsupported of ['scope_select_authority','scope_relocate','run_start','workspace_register']) assert.equal(tools.some(t=>t.name===unsupported),false);
          await m.close(); m=null;
        }
      }
      assert.deepEqual(fingerprint(f.root),before);
    }
  } finally {if(m)await m.close();f.close();}
});

test('separate Git directories are unsupported from primary and linked callers with no locator, fallback or writes', async () => {
  const f=new Fixture();let m;try{
    const primary=path.join(f.root,'separate-primary'), admin=path.join(f.root,'admin.git'), linked=path.join(f.root,'separate-linked');
    git(f.root,'init','--initial-branch=main','--separate-git-dir',admin,primary);
    git(primary,'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','--allow-empty','-m','seed');
    git(primary,'worktree','add','--detach',linked);
    const before=fingerprint(f.root);
    for(const selected of [primary,linked]){
      fail(selected,'unsupported_project','discover');
      fail(selected,'unsupported_project','item','list');
      fail(selected,'unsupported_project','item','create','--title','must not prepare');
      m=new MCP(selected);await m.init();await m.bad('discover',{},'unsupported_project');await m.bad('item_create',{title:'must not prepare'},'unsupported_project');await m.close();m=null;
    }
    assert.deepEqual(fingerprint(f.root),before);assert.equal(fs.existsSync(path.join(admin,'work-beta2')),false);
  }finally{if(m)await m.close();f.close();}
});

test('bounded hosting preserves prepared foreign-home ownership and rejects lost coordination without repairing it',async()=>{
  const f=new Fixture();let m;try{
    const a=f.create('boundary'), leaf=f.create('claimed child',a);f.commit();const destination=f.linked('destination');
    const run='11111111111141118111111111111111', session='22222222222242228222222222222222', acquisition='33333333333343338333333333333333';
    prepareRun(f.primary,'root',run);prepareNamedClaim(f.primary,'root',run,leaf,session,acquisition,'owner');
    const paths=[runPath(f.primary,'root',run,'run.yaml'),runPath(f.primary,'root',run,'sessions',`${session}.yaml`),runPath(f.primary,'root',run,'claims',`${acquisition}.yaml`)];
    const bytes=paths.map(p=>fs.readFileSync(p)), material=fs.readFileSync(itemPath(f.primary,leaf));
    const inventory=success(f.primary,'scope','inspect',a).scope.inventory_sha256, before=fingerprint(f.root);
    fail(f.primary,'claims_present','scope','host',a,'--destination',destination,'--expected-inventory',inventory);
    assert.deepEqual(fingerprint(f.root),before);
    success(f.primary,'scope','host',a,'--destination',destination,'--expected-inventory',inventory,'--executors-quiescent');
    m=new MCP(f.primary);await m.init();const owned=(await m.good('item_inspect',{id:leaf})).item;
    assert.equal(owned.current_claim.claim.id,acquisition);assert.equal(owned.current_claim.run_id,run);assert.equal(owned.context.effective_run_id,run);assert.equal(owned.context.source_worktree,destination);
    await m.bad('item_update',{id:leaf,title:'not authorized'},'claim_conflict');
    await m.good('item_update',{id:leaf,title:'owner write',session:'owner',claim_id:acquisition});
    for(let i=0;i<paths.length;i++)assert.deepEqual(fs.readFileSync(paths[i]),bytes[i]);
    assert.deepEqual(fs.readFileSync(itemPath(f.primary,leaf)),material);
    const lock=path.join(success(f.primary,'discover').git_common_dir,'work-beta2/coordination.lock');fs.unlinkSync(lock);
    const lost=fingerprint(f.root);assert.equal((await m.good('item_inspect',{id:leaf})).execution.state,'unavailable');
    await m.bad('item_ready',{},'coordination_unavailable');fail(f.primary,'coordination_unavailable','item','ready');
    await m.bad('item_update',{id:leaf,title:'must not repair ownership',session:'owner',claim_id:acquisition},'coordination_unavailable');
    assert.deepEqual(fingerprint(f.root),lost);assert.equal(fs.existsSync(lock),false);
  }finally{if(m)await m.close();f.close();}
});

test('a terminal selected root is unavailable, not an ended child barrier',async()=>{
  const f=new Fixture();let m;try{
    const id=f.create('still inspectable'), run='11111111111141118111111111111111';
    prepareRun(f.primary,'root',run);write(runPath(f.primary,'root',run,'run.yaml'),{format_version:2,id:run,created_at:epoch,phase:'finalized',ended_at:epoch});
    const before=fingerprint(f.root);m=new MCP(f.primary);await m.init();
    const view=await m.good('item_inspect',{id});assert.equal(view.execution.state,'unavailable');assert.equal(view.item.executable,null);
    assert.ok(view.diagnostics.some(d=>d.message.includes('terminal run')));
    await m.bad('item_ready',{},'invalid_source');await m.bad('item_update',{id,title:'blocked'},'invalid_source');
    assert.deepEqual(fingerprint(f.root),before);
  }finally{if(m)await m.close();f.close();}
});

test('an unreadable inactive scope directory does not suppress authoritative sibling scans',async()=>{
  const f=new Fixture();let m;try{
    const a=f.create('inactive primary scope'), leaf=f.create('authoritative root sibling');f.commit();const linked=f.linked('dedicated');host(f.primary,a,linked);
    copyScope(linked,f.primary,a);
    const bad=path.join(f.primary,'.work/scopes',a,'runs');fs.symlinkSync(path.join(f.root,'absent'),bad);
    const run='11111111111141118111111111111111';prepareRun(f.primary,'root',run);
    const manifest=fs.readFileSync(runPath(f.primary,'root',run,'run.yaml'));
    m=new MCP(f.primary);await m.init();const root=await m.good('scope_inspect',{scope:'root'});assert.equal(root.execution.state,'ready');
    const physical=await m.good('scope_inspect',{scope:a,view:'physical'});assert.ok(physical.diagnostics.some(d=>d.path===bad));
    await m.good('item_update',{id:leaf,title:'sibling routed'});assert.deepEqual(fs.readFileSync(runPath(f.primary,'root',run,'run.yaml')),manifest);
  }finally{if(m)await m.close();f.close();}
});

test('malformed inactive material filenames retain physical diagnostics without suppressing the scope tree',async()=>{
  const f=new Fixture();let m,bad,originalPath;try{
    const a=f.create('hosted boundary'), leaf=f.create('hosted leaf',a), sibling=f.create('primary sibling');f.commit();
    const linked=f.linked('hosted');host(f.primary,a,linked);
    const run='11111111111141118111111111111111';prepareRun(f.primary,'root',run);
    const authoritative=fs.readFileSync(itemPath(linked,a)), untouched=fs.readFileSync(itemPath(f.primary,sibling));
    bad=Buffer.concat([Buffer.from(path.join(f.primary,'.work/items','inactive-')),Buffer.from([0xff]),Buffer.from('.md')]);
    originalPath=itemPath(f.primary,a);fs.renameSync(originalPath,bad);
    const rawBad=fs.readFileSync(bad);
    assert.equal(success(f.primary,'item','inspect',leaf).item.context.source_worktree,linked);
    assert.equal(success(f.primary,'scope','inspect',a).scope.host,linked);
    m=new MCP(f.primary);await m.init();
    const root=await m.good('scope_inspect',{scope:'root'});assert.equal(root.execution.state,'ready');
    const selected=await m.good('item_inspect',{id:a});assert.equal(selected.item.context.source_worktree,linked);
    const physical=await m.good('scope_inspect',{scope:'root',view:'physical'});
    assert.ok(physical.diagnostics.some(d=>d.path?.includes('inactive-%FF.md')));
    await m.good('item_update',{id:leaf,title:'valid sibling mutation'});
    assert.deepEqual(fs.readFileSync(itemPath(linked,a)),authoritative);
    assert.deepEqual(fs.readFileSync(itemPath(f.primary,sibling)),untouched);
    assert.deepEqual(fs.readFileSync(bad),rawBad);
    // The same malformed entry is authoritative after explicit external restoration.
    // Required material guards must still refuse, without treating its parsed ID as absence.
    fs.renameSync(bad,itemPath(f.primary,a));bad=undefined;
    const activeBad=Buffer.concat([Buffer.from(path.join(f.primary,'.work/items','active-')),Buffer.from([0xfe]),Buffer.from('.md')]);
    fs.renameSync(itemPath(f.primary,sibling),activeBad);
    try{
      const physical=await m.good('item_diagnose',{view:'physical'});
      assert.ok(physical.diagnostics.some(d=>d.path?.includes('active-%FE.md')));
      await m.bad('item_update',{id:leaf,title:'blocked by authoritative damage'},'invalid_source');
      assert.deepEqual(fs.readFileSync(activeBad),untouched);
    }finally{fs.renameSync(activeBad,itemPath(f.primary,sibling));}
  }finally{
    if(m)await m.close();
    if(bad&&fs.existsSync(bad))fs.renameSync(bad,originalPath);
    f.close();
  }
});

test('raw inspection preserves unsafe and unreadable authoritative catalog failures on both views',async()=>{
  const f=new Fixture();let m;try{
    const id=f.create('catalog read provenance'), items=path.join(f.primary,'.work/items'), held=path.join(f.primary,'retained-items');
    const original=fs.readFileSync(itemPath(f.primary,id)), clean=fingerprint(f.root), mode=fs.statSync(items).mode&0o7777;
    m=new MCP(f.primary);await m.init();
    for(const damage of ['symlink','permission']){
      if(damage==='symlink'){fs.renameSync(items,held);fs.symlinkSync(held,items);}else fs.chmodSync(items,0);
      try{
        const code=damage==='symlink'?'unsafe_path':'io';
        for(const view of ['resolved','physical']){
          const normal=await m.bad('item_inspect',{id,view},code);
          const raw=await m.bad('item_inspect_raw',{id,view},code);
          const fromCli=fail(f.primary,code,'item','inspect',id,'--raw','--view',view);
          for(const error of [normal,raw,fromCli]){
            assert.equal(error.path,items);
            assert.ok(error.diagnostics.some(d=>d.path===items));
          }
        }
      }finally{
        if(damage==='symlink'){fs.unlinkSync(items);fs.renameSync(held,items);}else fs.chmodSync(items,mode);
      }
      assert.deepEqual(fs.readFileSync(itemPath(f.primary,id)),original);
      assert.deepEqual(fingerprint(f.root),clean);
    }
  }finally{if(m)await m.close();f.close();}
});

test('physical inspection retains local parsed and malformed pending progress without live authority',async()=>{
  const f=new Fixture();let m;try{
    const a=f.create('pending tree source'), parent=f.create('future parent');f.commit();const linked=f.linked('inactive-copy');
    host(f.primary,a,f.primary);
    const op='aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', anchor=path.join(f.primary,'.work/scopes/root/scope.yaml');
    const progress=path.join(f.primary,'.work/scopes/root/progress/start',op+'.yaml');
    const old=fs.readFileSync(itemPath(f.primary,a)), after=Buffer.from(old.toString().replace('---\n','---\nparent: "'+parent+'"\n'));
    const plan={format_version:2,id:op,created_at:epoch,scope_id:'root',anchor_before:{exists:true,sha256:digest(fs.readFileSync(anchor))},
      tree_change:{item_id:a,operation:'item_update',parent_before:null,parent_after:parent,old_parent_scope_id:'root',new_parent_scope_id:'root',item_before:digest(old),item_after:digest(after),step:0},
      tree_references:[],step:0};
    write(progress,plan);m=new MCP(f.primary);await m.init();
    const expected={id:op,kind:'start',path:progress,step:0,supported:true}, before=fingerprint(f.root);
    assert.deepEqual((await m.good('scope_inspect',{scope:'root'})).execution.pending,[expected]);
    for(const actual of [await m.good('scope_inspect',{scope:'root',view:'physical'}),success(f.primary,'scope','inspect','root','--view','physical')]){
      assert.deepEqual(actual.execution.pending,[expected]);assert.equal(actual.authoritative,false);
      assert.equal(actual.execution.coordination_available,false);assert.equal(actual.execution.state,'unavailable');
    }
    assert.deepEqual(fingerprint(f.root),before);
    // The same local information remains visible in an inactive physical checkout.
    fs.cpSync(path.join(f.primary,'.work/scopes/root'),path.join(linked,'.work/scopes/root'),{recursive:true});
    const copied=path.join(linked,'.work/scopes/root/progress/start',op+'.yaml');
    const physical=await m.good('scope_inspect',{scope:'root',worktree:linked,view:'physical'});
    assert.deepEqual(physical.execution.pending,[{...expected,path:copied}]);assert.equal(physical.authoritative,false);
    write(progress,'format_version: 2\nid: [broken\n');const brokenBefore=fingerprint(f.root);
    const malformed={id:null,kind:'start',path:progress,step:null,supported:false};
    for(const actual of [await m.good('scope_inspect',{scope:'root',view:'physical'}),success(f.primary,'scope','inspect','root','--view','physical')]){
      assert.deepEqual(actual.execution.pending,[malformed]);assert.ok(actual.diagnostics.some(d=>d.path===progress));
      assert.equal(actual.execution.coordination_available,false);
    }
    assert.deepEqual(fingerprint(f.root),brokenBefore);
  }finally{if(m)await m.close();f.close();}
});

function preparedHostPlan(f, a, destination, inventory, files) {
  const op = 'dddddddddddd4ddd8ddddddddddddddd';
  const boundaries = [{ scope_id: a, checkout: destination, anchor_before: { exists: false }, anchor_after: { format_version: 2, scope_id: a, execution: { mode: 'inherit' } }, parent_scope_id: 'root', child_before: { exists: false } }];
  if (!fs.existsSync(path.join(f.primary, '.work/scopes/root/scope.yaml'))) boundaries.unshift({ scope_id: 'root', checkout: f.primary, anchor_before: { exists: false }, anchor_after: { format_version: 2, scope_id: 'root', execution: { mode: 'uninitialized' } } });
  return { format_version: 2, id: op, created_at: epoch, scope_id: a, mode: 'host', source_hosts: [f.primary], destination, inventory: { scope_ids: [a], files, sha256: inventory }, declarations: [{ checkout: destination, before: { exists: false }, before_scope_ids: [], after: { format_version: 2, scope_ids: [a] } }], new_boundaries: boundaries, tree_references: [], executors_quiescent: false, step: 0 };
}

test('hosting continuations preserve inherited execution and refuse execution-changing boundary creations', async () => {
  for (const transport of ['cli', 'mcp']) {
    const f = new Fixture(); let m; try {
      const a = f.create('active inherited execution'); f.commit(); const destination = f.linked('destination');
      const run = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa'; prepareRun(f.primary, 'root', run);
      const item = success(f.primary, 'item', 'inspect', a).item; assert.equal(item.context.effective_run_id, run); assert.equal(item.context.execution_state, 'active');
      const inventory = success(f.primary, 'scope', 'inspect', a).scope.inventory_sha256;
      const plan = preparedHostPlan(f, a, destination, inventory, [{ path: `.work/items/${a}.md`, sha256: digest(fs.readFileSync(itemPath(f.primary, a))) }]);
      const progress = path.join(success(f.primary, 'discover').git_common_dir, 'work-beta2/relocations', `${plan.id}.yaml`);
      m = new MCP(f.primary); await m.init();
      for (const execution of [{ mode: 'ended', run_id: run }, { mode: 'boundary', run_id: run }]) {
        const bad = structuredClone(plan); bad.new_boundaries[0].anchor_after.execution = execution; write(progress, bad); const before = fingerprint(f.root);
        fail(f.primary, 'invalid_source', 'scope', 'host', a, '--destination', destination, '--expected-inventory', inventory, '--continue', plan.id);
        await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: plan.id }, 'invalid_source');
        assert.deepEqual(fingerprint(f.root), before); assert.deepEqual(JSON.parse(fs.readFileSync(progress)), bad);
      }
      write(progress, plan);
      const resumed = transport === 'cli' ? success(f.primary, 'scope', 'host', a, '--destination', destination, '--expected-inventory', inventory, '--continue', plan.id) : await m.good('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: plan.id });
      assert.equal(resumed.scope.host, destination); assert.equal(fs.existsSync(progress), false);
      const after = await m.good('item_inspect', { id: a }); assert.equal(after.item.context.effective_run_id, run); assert.equal(after.item.context.execution_state, 'active');
      assert.equal(success(f.primary, 'item', 'inspect', a).item.context.effective_run_id, run);
      assert.deepEqual(fs.readFileSync(itemPath(destination, a)), fs.readFileSync(itemPath(f.primary, a)));
    } finally { if (m) await m.close(); f.close(); }
  }
});

test('hosting continuations reject widened file marks and scope IDs retaining the old approved inventory digest', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('frozen inventory'); f.commit(); const destination = f.linked('destination');
    const inventory = success(f.primary, 'scope', 'inspect', a).scope.inventory_sha256;
    const plan = preparedHostPlan(f, a, destination, inventory, [{ path: `.work/items/${a}.md`, sha256: digest(fs.readFileSync(itemPath(f.primary, a))) }]);
    const progress = path.join(success(f.primary, 'discover').git_common_dir, 'work-beta2/relocations', `${plan.id}.yaml`);
    const b = 'bbbbbbbbbbbb4bbb8bbbbbbbbbbbbbbb'; const bytes = Buffer.from(front({ format_version: 1, id: b, title: 'unapproved descendant', completion: 'manual', state: 'open', priority: 2, parent: a }, '\nnew real material bytes'));
    m = new MCP(f.primary); await m.init();
    for (const variant of ['files', 'scope_ids']) {
      const bad = structuredClone(plan);
      if (variant === 'files') { for (const cwd of [f.primary, destination]) write(itemPath(cwd, b), bytes); bad.inventory.files.push({ path: `.work/items/${b}.md`, sha256: digest(bytes) }); }
      else bad.inventory.scope_ids.push(b);
      write(progress, bad); const before = fingerprint(f.root);
      fail(f.primary, 'conflict', 'scope', 'host', a, '--destination', destination, '--expected-inventory', inventory, '--continue', plan.id);
      await m.bad('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: plan.id }, 'conflict');
      assert.deepEqual(fingerprint(f.root), before); assert.deepEqual(JSON.parse(fs.readFileSync(progress)), bad);
      if (variant === 'files') for (const cwd of [f.primary, destination]) fs.unlinkSync(itemPath(cwd, b));
    }
    write(progress, plan); const resumed = await m.good('scope_host', { scope: a, destination, expected_inventory: inventory, continue_id: plan.id }); assert.equal(resumed.scope.host, destination); assert.equal(fs.existsSync(progress), false);
  } finally { if (m) await m.close(); f.close(); }
});

test('display prefixes disambiguate malformed canonical file identities in resolved and physical views', async () => {
  const f = new Fixture(); let m; try {
    const a = 'aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa', b = 'aaaaaaaabbbb4bbb8bbbbbbbbbbbbbbb';
    write(itemPath(f.primary, a), front({ format_version: 1, id: a, title: 'valid prefix', completion: 'manual', state: 'open', priority: 2 }, '\nverbatim valid body'));
    write(itemPath(f.primary, b), 'malformed canonical source without a header');
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root);
    for (const view of ['resolved', 'physical']) {
      const listed = success(f.primary, 'item', 'list', '--view', view).items.find(i => i.id === a), viaMCP = (await m.good('item_list', { view })).items.find(i => i.id === a);
      assert.equal(listed.display_id, 'w-aaaaaaaaa'); assert.equal(viaMCP.display_id, listed.display_id);
      assert.equal(success(f.primary, 'item', 'inspect', listed.display_id, '--view', view).item.id, a);
      assert.equal((await m.good('item_inspect', { id: viaMCP.display_id, view })).item.id, a);
      fail(f.primary, 'ambiguous_id', 'item', 'inspect', 'w-aaaaaaaa', '--view', view); await m.bad('item_inspect', { id: 'w-aaaaaaaa', view }, 'ambiguous_id');
    }
    assert.deepEqual(fingerprint(f.root), before);
    // Scope-only diagnostics are not extra candidates in the item-prefix resolver.
    fs.unlinkSync(itemPath(f.primary, b));
    write(path.join(f.primary, '.work/scopes/root/scope.yaml'), { format_version: 2, scope_id: 'root', execution: { mode: 'uninitialized' } });
    write(path.join(f.primary, '.work/scopes/root/children', `${b}.yaml`), { format_version: 2, child_scope_id: b });
    write(path.join(f.primary, '.work/scopes', b, 'scope.yaml'), { format_version: 2, scope_id: b, execution: { mode: 'inherit' } });
    const missingScopeBefore = fingerprint(f.root);
    for (const view of ['resolved', 'physical']) {
      const listed = success(f.primary, 'item', 'list', '--view', view).items.find(i => i.id === a);
      assert.equal(listed.display_id, 'w-aaaaaaaa');
      assert.equal((await m.good('item_list', { view })).items.find(i => i.id === a).display_id, listed.display_id);
      assert.equal(success(f.primary, 'item', 'inspect', listed.display_id, '--view', view).item.id, a);
      assert.equal((await m.good('item_inspect', { id: listed.display_id, view })).item.id, a);
    }
    assert.deepEqual(fingerprint(f.root), missingScopeBefore);
  } finally { if (m) await m.close(); f.close(); }
});

test('reserved same-directory stages never become live material candidates and reads preserve their bytes', async () => {
  const f = new Fixture(); let m; try {
    const a = f.create('only live source'), bytes = fs.readFileSync(itemPath(f.primary, a));
    for (const name of ['.stage-retained', '.operation-retained']) { const file = path.join(f.primary, '.work/items', name); write(file, bytes); fs.chmodSync(file, 0o600); }
    m = new MCP(f.primary); await m.init(); const before = fingerprint(f.root);
    for (const view of ['resolved', 'physical']) {
      const result = success(f.primary, 'item', 'list', '--view', view), rpc = await m.good('item_list', { view });
      assert.deepEqual(result.items.map(i => i.id), [a]); assert.deepEqual(rpc.items.map(i => i.id), [a]);
      assert.deepEqual(result.diagnostics, []); assert.deepEqual(rpc.diagnostics, []);
    }
    assert.deepEqual(fingerprint(f.root), before);
  } finally { if (m) await m.close(); f.close(); }
});
