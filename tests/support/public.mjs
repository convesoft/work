// Real public CLI/MCP acceptance. Fixtures are owned by the caller's TMPDIR.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createHash } from 'node:crypto';
const binary = process.env.WORK_TEST_BINARY;
assert.ok(binary && path.isAbsolute(binary), 'WORK_TEST_BINARY must identify the rebuilt executable');
assert.ok(process.env.TMPDIR && path.isAbsolute(process.env.TMPDIR), 'explicit disk-backed TMPDIR required');
const evidence = process.env.WORK_HOSTING_EVIDENCE;
function record(value) { if (evidence) fs.appendFileSync(evidence, `${JSON.stringify(value)}\n`); }
function execute(cwd, program, args) {
  const r = spawnSync(program, args, { cwd, encoding: 'utf8', env: process.env });
  record({ cwd, program, args, status: r.status, stdout: r.stdout, stderr: r.stderr });
  assert.equal(r.error, undefined); return r;
}
function git(cwd, ...args) { const r = execute(cwd, 'git', args); assert.equal(r.status, 0, r.stderr); return r.stdout.trim(); }
function cli(cwd, ...args) { const r = execute(cwd, binary, ['--json', ...args]); assert.equal(r.stderr, ''); const v = JSON.parse(r.stdout); return { status: r.status, ...v }; }
function success(cwd, ...args) { const r = cli(cwd, ...args); assert.equal(r.ok, true, JSON.stringify(r)); assert.equal(r.status, 0); return r.result; }
const errorSchema = JSON.parse(fs.readFileSync(new URL('../../docs/schemas/beta2-wire-v2.schema.json', import.meta.url), 'utf8')).$defs.Error;
function errorFields(error) { for (const key of Object.keys(error)) assert.ok(Object.hasOwn(errorSchema.properties, key), `undeclared Error field: ${key}`); for (const key of errorSchema.required) assert.ok(Object.hasOwn(error, key)); assert.ok(errorSchema.properties.code.enum.includes(error.code)); }
function fail(cwd, code, ...args) { const r = cli(cwd, ...args); assert.equal(r.ok, false); assert.equal(r.error.code, code, JSON.stringify(r)); errorFields(r.error); assert.notEqual(r.status, 0); return r.error; }
function write(file, value) { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, typeof value === 'string' || Buffer.isBuffer(value) ? value : `${JSON.stringify(value)}\n`); }
function digest(bytes) { return createHash('sha256').update(bytes).digest('hex'); }
function sourceDigest(p,s) {try{return digest(fs.readFileSync(p));}catch(e){if(e.code!=='EACCES')throw e;return `unreadable:${s.ino}:${s.size}:${s.mtimeMs}:${s.ctimeMs}`;}}
function fingerprint(root) {
  const files = []; function walk(dir) { let names;try{names=fs.readdirSync(dir).sort();}catch(e){if(e.code!=='EACCES')throw e;const s=fs.lstatSync(dir);files.push([path.relative(root,dir),s.mode,`unreadable-directory:${s.ino}:${s.size}:${s.mtimeMs}:${s.ctimeMs}`]);return;}for (const name of names) { const p = path.join(dir, name), s = fs.lstatSync(p); files.push([path.relative(root, p), s.mode, s.isSymbolicLink() ? fs.readlinkSync(p) : s.isFile() ? sourceDigest(p,s) : 'directory']); if (s.isDirectory()) walk(p); } }
  walk(root); return files;
}
class Fixture {
  constructor() { this.root = fs.mkdtempSync(path.join(process.env.TMPDIR, 'scope-hosting-')); this.primary = path.join(this.root, 'primary'); fs.mkdirSync(this.primary); git(this.primary, 'init', '--initial-branch=main'); git(this.primary, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-m', 'seed'); }
  commit() { git(this.primary, 'add', '.work/items'); git(this.primary, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'material'); }
  linked(name) { const p = path.join(this.root, name); git(this.primary, 'worktree', 'add', '--detach', p); return p; }
  create(title, parent) { return success(this.primary, 'item', 'create', '--title', title, '--body', '\nopaque α\n- [ ] not parsed\nlast', ...(parent ? ['--parent', parent] : [])).item.id; }
  close() { record({ cleanup: this.root, fingerprint: fingerprint(this.root) }); fs.rmSync(this.root, { recursive: true, force: false }); assert.equal(fs.existsSync(this.root), false); }
}
class MCP {
  constructor(cwd) { this.cwd = cwd; this.next = 1; this.pending = new Map(); this.stdout = ''; this.stderr = ''; this.child = spawn(binary, ['mcp'], { cwd, env: process.env }); this.child.stderr.on('data', d => { this.stderr += d; }); createInterface({ input: this.child.stdout }).on('line', line => { this.stdout += `${line}\n`; const v = JSON.parse(line); const p = this.pending.get(v.id); assert.ok(p); this.pending.delete(v.id); p(v); }); }
  async request(method, params = {}) { const id = this.next++; const req = { jsonrpc: '2.0', id, method, params }; record({ mcp_request: req, cwd: this.cwd }); const done = new Promise(resolve => this.pending.set(id, resolve)); this.child.stdin.write(`${JSON.stringify(req)}\n`); const result = await done; record({ mcp_response: result, cwd: this.cwd }); return result; }
  async init() { const r = await this.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'fixture', version: '1' } }); assert.equal(r.result.protocolVersion, '2025-06-18'); }
  async call(name, arguments_ = {}) { const r = await this.request('tools/call', { name, arguments: arguments_ }); assert.ok(r.result, JSON.stringify(r)); assert.equal(r.result.content[0].text, JSON.stringify(r.result.structuredContent)); return r.result; }
  async good(name, a = {}) { const r = await this.call(name, a); assert.equal(r.isError, false, JSON.stringify(r)); return r.structuredContent; }
  async bad(name, a, code) { const r = await this.call(name, a); assert.equal(r.isError, true); assert.equal(r.structuredContent.error.code, code, JSON.stringify(r)); errorFields(r.structuredContent.error); return r.structuredContent.error; }
  async close() { const exit = new Promise(resolve => this.child.once('exit', (code, signal) => resolve({ code, signal }))); this.child.stdin.end(); assert.deepEqual(await exit, { code: 0, signal: null }); assert.equal(this.stderr, ''); record({ mcp_complete_stdout: this.stdout, stderr: this.stderr, cwd: this.cwd }); }
}
function host(cwd, id, destination) { const i = success(cwd, 'scope', 'inspect', id); assert.ok(i.scope.inventory_sha256); return success(cwd, 'scope', 'host', id, '--destination', destination, '--expected-inventory', i.scope.inventory_sha256); }
function itemPath(cwd, id) { return path.join(cwd, '.work/items', `${id}.md`); }
function declaration(cwd) { return path.join(success(cwd, 'discover').git_dir, 'work-beta2-hosts.yaml'); }
function copyScope(source, destination, id) { fs.cpSync(path.join(source, '.work/scopes', id), path.join(destination, '.work/scopes', id), { recursive: true }); }

export { binary, record, execute, git, cli, success, fail, errorFields, write, digest, fingerprint, Fixture, MCP, host, itemPath, declaration, copyScope };
