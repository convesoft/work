// Tests the specification checker, not a beta.2 Work implementation or protocol.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { checkCatalog, checkFiles } from '../scripts/check-beta2-wire.mjs';

const catalogPath = new URL('../docs/schemas/beta2-wire-v2.schema.json', import.meta.url);
const documentPath = new URL('../docs/beta2-wire.mara.md', import.meta.url);
const catalog = JSON.parse(await readFile(catalogPath, 'utf8'));
const document = await readFile(documentPath, 'utf8');
const clone = () => structuredClone(catalog);

test('complete specification/catalog and canonical identity links agree', async () => {
  assert.deepEqual(await checkFiles(), { operations: 53, envelopes: 15, definitions: 168 });
});

test('template identifiers preserve unbounded lowercase vocabulary independently of session names', () => {
  const identifier = new RegExp(catalog.$defs.TemplateIdentifier.pattern, 'u');
  assert.ok(identifier.test('a'.repeat(256)));
  assert.ok(!identifier.test('Uppercase'));
  assert.ok(!new RegExp(catalog.$defs.Name.pattern, 'u').test('a'.repeat(256)));
  for (const name of ['RenderedItem', 'ExpandedItem']) {
    assert.deepEqual(catalog.$defs[name].properties.key, { $ref: '#/$defs/TemplateIdentifier' });
  }
  for (const name of ['TemplateValidateInput', 'TemplatePreviewInput', 'TemplateExpandInput']) {
    assert.deepEqual(catalog.$defs[name].properties.name, { $ref: '#/$defs/TemplateIdentifier' });
  }
  assert.equal(catalog.$defs.TemplateSummary.properties.name.type, 'string');
});

test('missing execution consumer is not mistaken for a complete matrix', () => {
  const incomplete = clone();
  delete incomplete.operations.claim_next;
  assert.throws(() => checkCatalog(incomplete, document), /tool coverage/);
});

test('undeclared fields, versions, and dangling schema references are rejected', () => {
  const open = clone();
  open.$defs.RunManifest.additionalProperties = true;
  assert.throws(() => checkCatalog(open, document), /open object/);
  const old = clone();
  old.$defs.ClaimAcquisition.properties.format_version.const = 1;
  assert.throws(() => checkCatalog(old, document), /ClaimAcquisition/);
  const dangling = clone();
  dangling.$defs.RunManifest.properties.id.$ref = '#/$defs/Absent';
  assert.throws(() => checkCatalog(dangling, document), /unresolved schema/);
});

test('unknown failures and grammar divergence fail consistency', () => {
  const unknown = clone();
  unknown.operations.run_finalize.errors.push('pretend_success');
  assert.throws(() => checkCatalog(unknown, document), /unknown error/);
  const grammar = clone();
  grammar.operations.run_rotate.cli = 'run rebind';
  assert.throws(() => checkCatalog(grammar, document), /CLI grammar mismatch/);
});

test('no obsolete alias or generic continuation consumer can enter the catalog', () => {
  const alias = clone();
  alias.operations.run_attach = alias.operations.run_start;
  assert.throws(() => checkCatalog(alias, document), /tool coverage/);
  const broad = clone();
  broad.$defs.ClaimRecoverInput.properties.continue_id = { $ref: '#/$defs/ID' };
  assert.throws(() => checkCatalog(broad, document), /continuation/);
});

test('onboarding has only top-level init/status and root-only configuration fingerprints', () => {
  assert.equal(catalog.operations.init.cli, 'init [--continue ID]');
  assert.equal(catalog.operations.status.cli, 'status [--view VIEW]');
  assert.ok(!catalog.operations.execution_init);
  assert.ok(!catalog.operations.execution_inspect);
  const missing = clone();
  missing.$defs.StartProgress.oneOf[0].allOf[0].then.required = [];
  assert.throws(() => checkCatalog(missing, document));
  const broad = clone();
  broad.$defs.StartProgress.oneOf[0].allOf[0].else.not = {};
  assert.throws(() => checkCatalog(broad, document));
});

test('hosting continuation preserves original declaration membership', () => {
  const missing = clone();
  missing.$defs.DeclarationChange.required = missing.$defs.DeclarationChange.required.filter(
    (field) => field !== 'before_scope_ids',
  );
  assert.throws(() => checkCatalog(missing, document));
  const references = clone();
  references.$defs.HostTransferProgress.required = references.$defs.HostTransferProgress.required.filter(
    (field) => field !== 'tree_references',
  );
  assert.throws(() => checkCatalog(references, document));
});

test('tree continuation freezes exact operation, material edge and every reference', () => {
  for (const field of ['operation', 'parent_before', 'parent_after']) {
    const missing = clone();
    missing.$defs.TreePlan.required = missing.$defs.TreePlan.required.filter((key) => key !== field);
    assert.throws(() => checkCatalog(missing, document));
  }
  const missing = clone();
  missing.$defs.StartProgress.oneOf[1].required = missing.$defs.StartProgress.oneOf[1].required.filter(
    (field) => field !== 'tree_references',
  );
  assert.throws(() => checkCatalog(missing, document));
});

test('disposal must preserve the explicit ended namespace barrier', () => {
  const removed = clone();
  removed.$defs.ExecutionBoundary.oneOf = removed.$defs.ExecutionBoundary.oneOf.filter(
    (variant) => variant.properties.mode.const !== 'ended',
  );
  assert.throws(() => checkCatalog(removed, document));
});
