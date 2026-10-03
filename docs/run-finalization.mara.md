:::mara design DES-FINALIZATION-API
:mid: 01M3TWRH58578K3JRDKZSTGQH4
:title: Define root digest extensions and explicit wisp disposal
:status: accepted
:kind: interface
:satisfies: REQ-RUN-FINALIZATION
:satisfies: REQ-OPAQUE-BODIES
:satisfies: REQ-EPHEMERAL-FILES

## Durable root extension

A digest is a document owned by the existing root, not a Work item: `.work/digests/<root-item-id>/<run-id>.md` in the captured surviving output workspace. Frontmatter has exactly `format_version: 1, root_item_id, run_id`; its Markdown body is the supplied summary bytes using normal item-style framing. It has no independent item ID, state, priority, parent edge or claim. Material root content/state are untouched. This separate extension file preserves opaque root-body bytes and avoids parsing or rewriting arbitrary Markdown sections.

Normal resolved root inspection adds `digests:[{run_id,path,body}]` from that root's source checkout, sorted by run ID; ordinary `body` remains the root's exact body. A run inspect/finalization result also identifies its output digest path. Output selection and any root source reassociation must be explicit when the intended surviving checkout differs; existing root binding must point to that output before squash. Never move the binding automatically or change root completion. Digests reach main by ordinary Git merge, just like material items. Fresh later runs naturally have distinct digest filenames.

## Bounded cleanup state

The run manifest is the sole cleanup progress record; no separate application or transaction receipt. Its optional `cleanup` has exactly `kind: squash|discard`, `finalize: boolean`, `item_ids: [wisp-id]`, `session_ids: [session-record-id]`, `started_at`. For squash, finalize must be true and output workspace is already in the manifest; for subset discard finalize is false and session_ids is empty. IDs name the deletion set, not copied entity payloads. Phase is squashing/discarding while cleanup exists; finalized/disposed retains the cleanup mapping plus ended_at for inspection. An active run has no cleanup. Terminal records cannot be resumed or modified except exact repeated completion responses.

Before any deletion, validate the whole selected set, no active claims on target work, and no references from surviving items or needed handoffs into that set. Full cleanup additionally refuses active claims anywhere in the run. Examine current resolved items, not every historical Git branch or ended-claim snapshot. References wholly within the set do not block. Prune handoffs with all receivers resolved before this check; handoffs still needed outside the set block and return their IDs. Do not silently rewrite an outside dependency or cascade to material work.

Explicit discard may abandon receivers wholly within the deletion set. An unresolved handoff blocks deletion only when it references that set and has an unresolved receiver outside it. Retain handoffs whose unresolved receivers are all being discarded as independent files; do not mark those receivers done or extend run cleanup to handoff deletion. Subsequent handoff inspection/pruning reports missing receivers and retains the context. This user-selected retention decision is flagged for later product review; automatic orphan-handoff recovery remains outside beta finalization.

Save the manifest's bounded cleanup set under the shared lock, freezing membership/template expansion and new claims for that run. While squashing, also refuse item mutations and material-source rebinding for run members, and refuse outside item/graph changes that could alter their derived completion (including changing aggregate descendants or child membership). Apply this to ordinary item operations, physical repair and template expansion. The root is not automatically frozen: its independent lifecycle and explicit source binding remain available unless an authored graph change affects a frozen member's completion. This preserves the finished member obligations captured before cleanup, even when interruption removes some completed child files; do not reconstruct deleted wisps or add a copied completion overlay. Terminal repeats remain read-only outcomes. Direct edits and external Git changes do not cooperate with Work's locks and remain outside this freeze guarantee; detected source conflicts still refuse cleanup. For squash, create the digest as create-new, or accept an existing file only when root/run header and supplied body exactly match. Conflicting content refuses; retain it for inspection. Only after the digest is safely retained delete wisps and named-session files in ID order, syncing each deletion. Mark finalized after completion. Missing files from that recorded set are already-deleted on retry; unexpected paths/entities are never swept up. Material members, their bindings, shared workspace files and historical claims remain.

For discard, the same ordering omits the digest write entirely. Subset success clears cleanup and returns phase active; full success marks disposed. Full disposal removes only run-owned wisps/sessions, not material files, material bindings or useful outside handoffs. A partial deletion error exposes deleted/remaining IDs and keeps the manifest phase; repeat the same operation/set to continue. This is bounded deletion progress in an existing run entity, not general batch rollback. No automatic timeout, age-based deletion, backup, promotion or generic repair engine.

## Operations and outcomes

| CLI | MCP | Request | Result |
| --- | --- | --- | --- |
| `run squash RUN --summary TEXT|-` | `run_squash` | `run_id, summary` | `{run_id,phase,digest_path,deleted,changed}` |
| `run discard RUN --all` | `run_discard` | `run_id, all:true` | `{run_id,phase,deleted,changed}` |
| `run discard RUN --item ITEM...` | `run_discard` | `run_id, items:[id]` nonempty | same |

Exactly one of all or items is supplied. Subset items must be wisps of that run; a material ID is `invalid_argument`, never a request to delete it. Squash requires finished member work and no active claims. Discard may abandon unfinished wisps, but still refuses active target claims. It creates no digest, reports disposal rather than success of those obligations, and does not complete the root. An empty current run can be explicitly discarded but is not squash-finished.

For a pending squash retry, require the same summary and verify any existing digest; if interrupted before digest creation, the caller resupplies it. Terminal squash repeated with matching body returns changed:false and the existing path. A different body conflicts. Pending discard requires exactly the stored ID set/finalize mode (an all request means that recorded full set while pending); incompatible requests return run_conflict. Terminal full discard repeated returns changed:false. Repeating a completed subset discard with now-missing IDs returns not_found; inspect the prior result instead of demanding a retry receipt. Lifecycle roots remain independent throughout.

## Acceptance

Test finished-without-cleanup retention, digest text/body preservation, no new digest task, repeated squash, material-output worktree selection, and interruption before digest creation and between deletion steps. Test selected/full discard with open and closed wisps, no summary, unchanged material/root bytes, preserved bindings, active-claim refusal, incoming references, inside-set references, named-session cleanup and same-run continuation. After finalized/disposed, a new run gets a fresh ID. Test through CLI/MCP with disposable linked worktrees and one cross-filesystem output case where supported; no untested power-loss claim.
:::
