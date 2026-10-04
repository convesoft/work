#!/usr/bin/env node
// Declarative specification checks only. This never runs Work or mutates its state.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const tools = [
  'discover', 'execution_inspect', 'execution_init',
  'scope_list', 'scope_inspect', 'scope_host', 'scope_relocate', 'scope_adopt',
  'scope_select_authority', 'scope_dispose',
  'run_start', 'run_inherit', 'run_rotate', 'run_inspect', 'run_list',
  'run_finalize', 'run_abandon', 'run_dispose',
  'session_register', 'session_inspect', 'session_list', 'session_retire', 'session_observe',
  'claim_acquire', 'claim_next', 'claim_inspect', 'claim_list', 'claim_release',
  'claim_recover', 'claim_reassign',
  'handoff_create', 'handoff_inspect', 'handoff_list', 'inbox',
  'item_create', 'item_list', 'item_inspect', 'item_inspect_raw', 'item_diagnose',
  'item_ready', 'item_update', 'item_close', 'item_reopen', 'item_repair',
  'item_related', 'item_explain', 'item_promote', 'relation_add', 'relation_remove',
  'template_list', 'template_validate', 'template_preview', 'template_expand',
].sort();
const envelopes = [
  'ScopeAnchor', 'ChildReference', 'HostDeclaration', 'RunManifest', 'NamedSession',
  'ClaimAcquisition', 'ClaimEnding', 'HandoffHeader', 'DigestHeader',
  'StartProgress', 'EndingProgress', 'RotationProgress', 'PromotionProgress',
  'DisposalProgress', 'HostTransferProgress',
].sort();
const removed = [
  'workspace_register', 'workspace_bind', 'workspace_unbind', 'run_attach', 'run_detach',
  'session_set', 'session_remove', 'handoff_receivers', 'handoff_prune',
  'run_squash', 'run_discard', 'storage_recreate', 'storage_recover',
];
const continuation = new Set([
  'execution_init', 'run_start', 'run_finalize', 'run_abandon', 'run_rotate',
  'run_dispose', 'scope_dispose', 'item_promote', 'item_update',
  'relation_add', 'relation_remove', 'scope_host', 'scope_relocate',
  'scope_adopt', 'scope_select_authority',
]);

function walk(value, visit, location = '$') {
  if (!value || typeof value !== 'object') return;
  visit(value, location);
  for (const [key, child] of Object.entries(value)) {
    walk(child, visit, `${location}/${key}`);
  }
}

export function checkCatalog(catalog, document) {
  const definitions = catalog.$defs;
  const resolve = (reference) => {
    assert.match(reference, /^#\/\$defs\/[A-Za-z]+$/);
    const name = reference.slice('#/$defs/'.length);
    assert.ok(definitions[name], `unresolved schema: ${reference}`);
    return definitions[name];
  };
  assert.equal(catalog.$schema, 'https://json-schema.org/draft/2020-12/schema');
  assert.deepEqual(Object.keys(catalog.operations).sort(), tools, 'tool coverage');
  assert.deepEqual([...catalog.envelopes].sort(), envelopes, 'envelope coverage');
  assert.deepEqual(catalog.oneOf.map((entry) => entry.$ref.slice('#/$defs/'.length)).sort(), envelopes);
  walk(definitions, (schema, location) => {
    if (schema.$ref) resolve(schema.$ref);
    // Only actual object schemas, not condition/constraint fragments, require closure.
    if (schema.type === 'object' && schema.properties) {
      assert.equal(schema.additionalProperties, false, `open object: ${location}`);
      assert.ok(Array.isArray(schema.required), `missing required list: ${location}`);
      assert.equal(new Set(schema.required).size, schema.required.length, location);
      for (const key of schema.required) assert.ok(key in schema.properties, `${location}: ${key}`);
    }
  });
  const rows = new Map();
  for (const line of document.split('\n')) {
    const match = line.match(/^\| `([^`]+)` \| `([a-z_]+)` \|/);
    if (!match || !tools.includes(match[2])) continue;
    assert.ok(!rows.has(match[2]), `duplicate matrix row: ${match[2]}`);
    rows.set(match[2], match[1]);
  }
  assert.deepEqual([...rows.keys()].sort(), tools, 'document/tool matrix coverage');
  const codes = new Set(definitions.Error.properties.code.enum);
  assert.deepEqual(Object.keys(catalog.error_exit_codes).sort(), [...codes].sort());
  for (const [tool, operation] of Object.entries(catalog.operations)) {
    assert.equal(operation.cli, rows.get(tool), `CLI grammar mismatch: ${tool}`);
    const input = resolve(operation.input.$ref);
    resolve(operation.output.$ref);
    assert.equal(input.type, 'object');
    assert.equal(input.additionalProperties, false);
    assert.ok(input.properties.worktree, `${tool}: explicit checkout missing`);
    assert.equal(Boolean(input.properties.continue_id), continuation.has(tool), `${tool}: continuation`);
    assert.ok(operation.errors.length > 0, `${tool}: no typed errors`);
    assert.equal(new Set(operation.errors).size, operation.errors.length);
    for (const code of operation.errors) {
      assert.ok(codes.has(code), `${tool}: unknown error ${code}`);
      assert.ok(Number.isInteger(catalog.error_exit_codes[code]));
    }
    for (const code of catalog.common_errors) assert.ok(operation.errors.includes(code));
  }
  for (const name of envelopes) {
    const variants = definitions[name].oneOf ?? [definitions[name]];
    for (const variant of variants) {
      assert.equal(variant.properties.format_version.const, 2, name);
      assert.ok(variant.required.includes('format_version'), name);
      assert.equal(variant.additionalProperties, false, name);
    }
    assert.ok(document.includes(`\`${name}\``), `undocumented envelope ${name}`);
  }
  for (const name of removed) assert.ok(!(name in catalog.operations), `obsolete tool ${name}`);
  const properties = (name) => Object.keys(definitions[name].properties).sort();
  assert.deepEqual(properties('HostDeclaration'), ['format_version', 'scope_ids']);
  assert.deepEqual(properties('ScopeAnchor'), ['execution', 'format_version', 'scope_id']);
  assert.ok(!properties('RunManifest').includes('material_items'));
  assert.deepEqual(definitions.ScopeAnchor.properties.scope_id, { $ref: '#/$defs/ScopeID' });
  assert.deepEqual(definitions.ClaimAcquisition.properties.identity, { $ref: '#/$defs/External' });
  assert.deepEqual(definitions.ClaimEnding.properties.recovery, { const: true });
  assert.deepEqual(definitions.ClaimRecoverInput.properties.executors_stopped, { const: true });
  assert.deepEqual(definitions.ClaimReassignInput.properties.executors_stopped, { const: true });
  assert.deepEqual(definitions.AcquisitionResult.properties.claim, { $ref: '#/$defs/ClaimView' });
  assert.ok(definitions.ClaimNextResult.properties.claim.anyOf.some((entry) => entry.type === 'null'));
  for (const tool of ['claim_acquire', 'claim_next', 'claim_reassign']) {
    assert.ok(resolve(catalog.operations[tool].input.$ref).allOf.some((rule) => rule.oneOf));
  }
  for (const tool of ['item_update', 'item_close', 'item_reopen', 'item_repair', 'relation_add', 'relation_remove']) {
    const input = resolve(catalog.operations[tool].input.$ref);
    assert.ok(input.properties.authorization, `${tool}: multi-source authorization`);
    assert.ok(input.allOf.some((rule) => rule.oneOf), `${tool}: mixed owner form rejection`);
  }
  assert.equal(definitions.TemplateIdentifier.pattern, '^[a-z][a-z0-9_]*$');
  for (const name of ['RenderedItem', 'ExpandedItem']) {
    assert.deepEqual(definitions[name].properties.key, { $ref: '#/$defs/TemplateIdentifier' });
  }
  for (const name of ['TemplateValidateInput', 'TemplatePreviewInput', 'TemplateExpandInput']) {
    assert.deepEqual(definitions[name].properties.name, { $ref: '#/$defs/TemplateIdentifier' });
  }
  // The ended barrier is retained even after the terminal manifest is disposed.
  assert.ok(definitions.ExecutionBoundary.oneOf.some((variant) => variant.properties.mode.const === 'ended'));
  return { operations: tools.length, envelopes: envelopes.length, definitions: Object.keys(definitions).length };
}

export async function checkFiles() {
  const [catalogText, document, scoped, status] = await Promise.all([
    readFile(path.join(root, 'docs/schemas/beta2-wire-v2.schema.json'), 'utf8'),
    readFile(path.join(root, 'docs/beta2-wire.mara.md'), 'utf8'),
    readFile(path.join(root, 'docs/scoped-execution.mara.md'), 'utf8'),
    readFile(path.join(root, 'docs/engineering-status.mara.md'), 'utf8'),
  ]);
  const result = checkCatalog(JSON.parse(catalogText), document);
  const wire = scoped.match(/:::mara design DES-BETA2-WIRE\n([\s\S]*?)\n:::/)?.[1];
  assert.ok(wire, 'canonical wire item missing');
  assert.match(wire, /:mid: 01M439Z97QQ2N76958306N0TJR/);
  assert.match(wire, /:status: (draft|accepted)\n/);
  assert.ok(wire.includes('beta2-wire.mara.md'), 'canonical wire body must link full specification');
  assert.ok(status.includes('beta2-wire.mara.md'), 'status must link full specification');
  assert.ok(document.includes('not runtime') || document.includes('not planned runtime'));
  return result;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await checkFiles();
    console.log(JSON.stringify({ ok: true, kind: 'specification_consistency', ...result }));
  } catch (error) {
    console.error(error.stack);
    process.exitCode = 1;
  }
}
