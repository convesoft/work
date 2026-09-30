# Remaining engineering details and risks

The product model and initial scope are established. The remaining work below concerns engineering contracts and small interface conventions, not another workflow or orchestration layer. Record concrete decisions in Mara as those contracts are designed; keep unresolved details distinct from accepted behavior.

## Engineering contracts to finish

| Choice | What remains unresolved | Affected contract |
| --- | --- | --- |
| File formats | Item/template formats are defined. The entity layout and lifetimes are settled; versioned envelopes for claims, runs, applications, sessions, workspaces, handoffs and operation receipts still need exact fields and ID rules | [[DES-ENTITY-LIFECYCLES]], [[DES-ITEM-FORMAT]], [[DES-TEMPLATE-FORMAT]] |
| Worktree views and claims | Source-change detection, claim mutation integration with checkout-local locks, and representation of the surviving output checkout. Path selection, one run per item and repository-wide exclusion are settled | [[DES-FILE-COORDINATION]], [[REQ-SINGLE-RUN]], [[REQ-WORKTREE-VIEWS]] |
| Ownership recovery | Inputs and file-recovery transitions for explicit release/reassignment, trusted restore and recreation. Distinguish fresh initialization from detectable storage loss; specify retained identity and former-token invalidation. No automatic expiry or mandatory heartbeat | [[REQ-CLAIM-RECOVERY]], [[DES-FILE-COORDINATION]] |
| Session reuse | Assignment-hint and optional availability-observation serialization. Names are run-scoped and removed during successful finalization cleanup; one-time workers need no named registration | [[DES-CLAIM-CONTEXT]] |
| Workspace cleanup protocol | Cleanup-target representation and recovery after physical deletion but before reporting success. Check users and mark closing before removal, reject new assignments, execute from a surviving checkout, and retain failure context | [[REQ-WORKSPACE-CLEANUP]], [[DES-WORKSPACE-ASSOCIATIONS]] |
| Run file protocol | Exact manifest/application envelopes, committed visibility point, staged operation transitions and idempotency receipts. The run is finished when its obligations are resolved and no claims remain; review progress uses ordinary items and edges | [[REQ-RUN-ATOMICITY]], [[DES-TEMPLATE-RUNS]], [[DES-FILE-COORDINATION]] |
| Squash and cleanup | Idempotent digest publication and interruption recovery. Use a surviving checkout and refuse deletion when outside references still require temporary records; do not rewrite those references automatically | [[REQ-RUN-FINALIZATION]], [[DES-SQUASH-DIGEST]] |
| Handoff operations | Concurrent receiver changes and recoverable save/release/cleanup. Cancellation resolves a receiver; reopening does not resurrect deleted notes | [[DES-HANDOFF-RECORDS]] |
| Selection metadata | Scope/filter operation schema remains open; [[DES-TEMPLATE-FORMAT]] settles model/thinking template defaults. Version 1 defines priority 0–4 with ID tie-breaking, exact case-sensitive labels, and no parent inheritance | [[DES-ITEM-FORMAT]], [[REQ-WORK-SELECTION]], [[REQ-EXECUTOR-HINTS]] |
| Operation schemas | The first durable CLI and MCP slice is defined in [[DES-CLI-JSON]] and [[DES-MCP-STDIO]]; [[DES-TEMPLATE-FORMAT]] defines template discovery, validation, and preview. Later claim and run schemas, pagination, and compatibility guarantees remain | [[REQ-CLI-MCP-PARITY]], [[REQ-CLI-JSON]] |
| Graph errors | Diagnostic representation and repair interfaces. An invalid selected graph blocks readiness and claims; inspection/repair and other projects remain available | [[REQ-GRAPH-INTEGRITY]] |
| Repository storage | Per-entity format compatibility, safe initialization, lock/error interfaces, backups and recovery-copy/receipt retention. All authority is in plain files; linked worktrees share coordination, independent clones do not | [[DES-SHARED-FILES]], [[DES-ENTITY-LIFECYCLES]], [[REQ-GIT-CHECKOUT]] |

The confirmed initial scope is recorded in [[ADR-INITIAL-SCOPE]]. [[ADR-RELATION-SEMANTICS]] settles the four relationships, parent completion policies, inherited prerequisites, and lifecycle versus informational distinction. Built-in agent execution, worktree management, and PR/CI actions are outside the initial scope. A TUI remains a later product direction. Multi-machine synchronization, a hosted service, arbitrary workflow scripting, and automatic provider polling have no accepted requirements in this corpus.

The structured external-outcome evaluator proposed in [[REQ-GATE-CONTEXT]] and [[VER-GATE-REVISION]] is retired. External agents interpret review and CI information and add ordinary work and blocking edges; Work does not need a result classifier or revision-aware gate engine. Detailed protocols must preserve that small model.

Snooze/defer scheduling, arbitrary promotion of temporary items, general batch editing, and shared-resource locks are deferred. Template publication and squash still need narrow recoverable multi-file operations. Five fixes followed by consolidation use ordinary dependencies and handoff context; no resource-lock or orchestration primitive is required for that sequence.

[[DES-LIFECYCLE]] settles closure, cancellation, and reopening: cancellation closes an obligation with a reason; reopening recalculates readiness and aggregate state without cascading into completed manual work or restoring discarded operational context. [[DES-ITEM-FORMAT]] settles item serialization, identity, labels, and priority conventions. Unfinished-run discard is outside the defined squash operation and does not need to expand the initial workflow model.

## Material risks

:::mara risk RISK-DIVERGENT-VIEWS
:mid: 01M3KAW7AX5F7AF33GXDF9955F
:title: Shared indexing can mix branch-local truth
:status: accepted
:treatment: open
:affects: REQ-DURABLE-FILES
:affects: DES-SHARED-FILES

Linked worktrees can hold different versions of the same durable item. A global cache keyed only by item ID, or a shared completed flag treated as durable truth, could leak one branch's state into another and schedule work incorrectly. Keep selected-checkout source views distinct from shared ownership and execution context under [[DES-SHARED-FILES]]. Resolve source-change detection before implementing cross-worktree selection. Obligation: [[REQ-WORKTREE-VIEWS]].
:::

:::mara risk RISK-FILE-DB-CONSISTENCY
:mid: 01M3KAW7B7E9QCWDXAHNGK897V
:title: Interrupted multi-file updates can lose or duplicate work
:status: accepted
:treatment: open
:affects: DES-PERSISTENCE-BOUNDARY
:affects: DES-EPHEMERAL-LAYOUT

Separate entity files have separate atomicity boundaries. A crash during run publication, completion with handoff/claim release, or digest finalization can expose partial work or lose useful results even while a global lock excludes other writers. Specify staged publication with a commit/visibility point, recoverable operation records, source conflict handling, idempotent retry and reference-safe cleanup. [[DES-FILE-COORDINATION]] defines the boundary; [[REQ-RUN-ATOMICITY]] and [[REQ-RUN-FINALIZATION]] require item-specific protocols before implementation. The stable risk ID is retained from the earlier file/database design.
:::

:::mara risk RISK-STALE-COORDINATION
:mid: 01M3KAW7BG8DYZMDFABEMN939V
:title: Lost or stale ownership can allow duplicate execution
:status: accepted
:treatment: open
:affects: REQ-SIDE-STATE
:affects: ADR-INITIAL-SCOPE

A crashed agent, stale claim, or manual operational-file deletion or storage reset can leave an old executor working while a new agent claims the same item. Preserved item bodies do not reconstruct lost claim files. A plain-file layout remains vulnerable to edits outside the shared lock. Define claim identity, fencing of former owners, recovery behavior, and operator-visible uncertainty. Work cannot stop an external agent merely by changing its recorded claim. Proposed obligations: [[REQ-CLAIM-EXCLUSION]] and [[REQ-CLAIM-RECOVERY]].
:::
