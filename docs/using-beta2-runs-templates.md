# Local beta.2 runs and templates

This is the **local issue-branch** surface, not the published beta.1 package or a claim of scope integration. It adds eight operations to the bounded hosting/initialization/material surface. Cargo/npm versions remain unchanged. The complete catalog in [the accepted wire specification](beta2-wire.mara.md) is larger than the delivered subset.

## Explicit child rounds

```sh
work init                         # the separate initializer: one standing root run
work run start ITEM                # a material item's own execution boundary
work run inspect RUN_ID
work run list
work run inherit ITEM              # resume the ancestor namespace after an ended child
```

`start` excludes virtual `root` and wisps. It never ends an active round: repeating a healthy current start is unchanged. A fresh start after a retained ended child chooses a new run identity; historical wisps stay in their old home. Inheritance does not end an active child and preserves any material hosting override. Hosting alone inherits execution; only an own run gives a new namespace.

Membership is derived from declared ancestry, not an attach/detach list. The run root has context but its own completion is excluded from member-finished calculation. A non-root round needs nonempty completed required work, including descendant work, with relevant item/root/session claims drained. The standing root's `finished` is null: it does not assert that the repository has no work. Finalization would also need ended child rounds; it is not delivered here.

A stored boundary above existing stored descendants freezes all required parent-local reference changes before publication. A reported interrupted start is continued only by the original caller and exact scope/operation ID:

```sh
work status
work run start ITEM --continue OPERATION_ID
```

Continuation checks the frozen source snapshots, original run identity and complete reference target set. Complete operation-owned staging bytes are reconciled only when they exactly match the frozen next publication. Edited, incomplete or unknown stages remain conflicts with recovery evidence. A saved manifest justified by the authoritative pending start is available to run inspect/list before the selecting anchor exists; it is not current or executable. Arbitrary unreferenced run copies are not inferred as authoritative. Completed progress has no permanent retry receipt. Do not repeat a failed multi-file operation blindly.

## Selected definitions, authoritative destinations

Definitions are `.work/templates/NAME.yaml` in the **selected checkout**. Their v1/v2 parser, scalar parameter declarations, exact bindings, single-pass substitution, opaque bodies and item/default hint provenance remain supported.

```sh
work template list
work template validate plan
work template preview plan --param subject=BUG-7
work template expand plan --param subject=BUG-7
work template preview plan --root ITEM --existing delivery=FULL_ITEM_ID
work template expand plan --scope SCOPE --run RUN_ID --root ITEM
```

Repeated `--param NAME=TEXT` and `--existing NAME=FULL_ID` split only at the first `=`. Duplicate keys, unknown declarations and non-full existing IDs refuse. `root` is a template graph endpoint, **not** an automatic parent or execution selector. If `--scope` and `--run` are both supplied, they must agree. Explicit parent edges must connect child-run wisps beneath the run's material scope. Root-run parentless wisps need no synthetic root item/parent.

Material-only preview/expansion works before execution initialization, including a fresh template-only clone. Empty list reads do not bootstrap anything. Preview writes nothing and allocates no permanent IDs. With no active execution target it can show symbolic wisps with `destination:null` and `placement_validated:false`. Physical preview is read-only, nonauthoritative and does not promise executable placement.

Material files follow prospective ancestry and the nearest authoritative host. Wisps go to the one owning run home, even when a hosting-only child uses another checkout. A material/wisp mixture is validated against the complete graph, all destinations and current ownership before publication. Claims authorize exact existing source rewrites; context changes cannot move current ownership or current wisps between runs. Same-run wisp reparenting preserves its actual run-home path and maintains any descendant stored-scope references through the existing frozen tree operation. It does not transfer surviving material sources to another host.

## Per-file publication and recovery

Expansion allocates IDs in memory, publishes new final documents in local-key order, then rewrites affected existing sources in canonical path order (with needed parent-local reference maintenance). It is **not atomic**. It creates no template application, journal, member registry, rollback or deduplication key. A reported failure exposes allocated key/ID mapping, known created/updated paths and applicable uncertain/surviving recovery paths. Published files remain after a failure. A broken partial graph is not claimable. A crash can lose the in-memory ID mapping; inspect ordinary files. After caller-directed repair/cleanup, fresh retry allocates new IDs.

Verified ordinary publication cleans obsolete displaced stages. Unknown/conflicting stages and bytes still needed by frozen continuation are retained, never removed by name/age. Errors after publication remain errors with partial evidence; a cleanup failure is not permission to replay unfinished work. Locks exclude cooperating callers, and captured bytes/inodes/modes/directories/routing are rechecked. Raw editors, external Git and stale open handles do not obey these locks. This is not cross-file atomicity or a power-loss guarantee.

## Deliberate boundary

Delivered CLI/MCP operations: `run_start`, `run_inherit`, `run_inspect`, `run_list`, `template_list`, `template_validate`, `template_preview`, `template_expand`.

Rotation **preparation only** is internal/read-only: refuse live ownership (including root-owned names claiming in other runs), active descendants and unfinished root wisps without explicit abandonment. Public `run_rotate` is absent. No ending/replacement is performed by preparation. Named-session/claim acquisition/runtime, attached handoffs, finalization/abandonment, promotion, relocation, disposal and final installed-package/combined acceptance remain separately owned. Prepared records test guards only, not those commands.

See [test migration](../tests/MIGRATION.md) for the active public counterparts and still-unexecuted legacy/downstream obligations.
