# Remaining engineering details and risks

The product model and initial scope are established. The remaining work below concerns engineering contracts and small interface conventions, not another workflow or orchestration layer. Record concrete decisions in Mara as those contracts are designed; keep unresolved details distinct from accepted behavior.

## Engineering contracts to finish

| Choice | What remains unresolved | Affected contract |
| --- | --- | --- |
| File formats | Item/template formats are defined. The entity layout and lifetimes are settled; exact envelopes for emitted claims and retirement, runs and active execution state, sessions, workspaces and handoffs still need fields and ID rules. The per-item material/wisp selector is a planned template extension; it is not implemented in current preview | [[DES-ENTITY-LIFECYCLES]], [[DES-ITEM-FORMAT]], [[DES-TEMPLATE-FORMAT]] |
| Worktree views and claims | Source-change detection, claim mutation integration with checkout-local locks, and representation of the surviving output checkout. Selected durable definitions plus shared active-run state, path selection, one run per item and repository-wide exclusion are settled | [[DES-FILE-COORDINATION]], [[REQ-SINGLE-RUN]], [[REQ-WORKTREE-VIEWS]] |
| Ownership recovery | Foundation initialization, detectable loss, explicit recreation and status-only scope are defined in [[DES-STORE-FOUNDATION]] and [[DES-STORAGE-API]]. Claim ID plus session identity replaces separate ownership tokens. New acquisitions retain distinct claim identities; exact release/retirement encoding remains to be defined. Backup/restore is deferred until relevant entity formats exist. No automatic expiry or mandatory heartbeat | [[REQ-CLAIM-RECOVERY]], [[DES-FILE-COORDINATION]] |
| Session reuse | Assignment-hint and optional availability-observation serialization. Names are run-scoped and removed during successful finalization cleanup; one-time workers need no named registration | [[DES-CLAIM-CONTEXT]] |
| Workspace cleanup protocol | Cleanup-target representation and recovery after physical deletion but before reporting success. Check users and mark closing before removal, reject new assignments, execute from a surviving checkout, and retain failure context | [[REQ-WORKSPACE-CLEANUP]], [[DES-WORKSPACE-ASSOCIATIONS]] |
| Run file protocol | Exact run/state envelopes and CLI/MCP creation results. Mixed templates create material items in the selected checkout and wisps inside a run; material-only expansion needs no run. No application records or atomic multi-file publication: agents inspect partial files, remove them and retry. A run finishes when member obligations resolve and no active claims remain; the root is separate | [[REQ-RUN-ATOMICITY]], [[DES-TEMPLATE-RUNS]], [[DES-FILE-COORDINATION]] |
| Squash and cleanup | Exact representation and repeat-call behavior of the caller-authored root digest extension. Squash retains it before cleanup without creating a digest item or closing the root. Explicit discard needs no digest and may abandon unfinished wisps; its exact CLI/MCP and disposal representation remain to be specified. Both paths refuse active target claims and outside references needing the deletion set | [[REQ-RUN-FINALIZATION]], [[DES-SQUASH-DIGEST]] |
| Handoff operations | Concurrent receiver changes and recoverable save/release/cleanup. Cancellation resolves a receiver; reopening does not resurrect deleted notes | [[DES-HANDOFF-RECORDS]] |
| Selection metadata | Scope/filter operation schema remains open; [[DES-TEMPLATE-FORMAT]] settles model/thinking template defaults. Version 1 defines priority 0–4 with ID tie-breaking, exact case-sensitive labels, and no parent inheritance | [[DES-ITEM-FORMAT]], [[REQ-WORK-SELECTION]], [[REQ-EXECUTOR-HINTS]] |
| Operation schemas | The first durable CLI and MCP slice is defined in [[DES-CLI-JSON]] and [[DES-MCP-STDIO]]; [[DES-TEMPLATE-FORMAT]] defines template discovery, validation, and preview. Later claim and run schemas, pagination, and compatibility guarantees remain | [[REQ-CLI-MCP-PARITY]], [[REQ-CLI-JSON]] |
| Graph errors | Diagnostic representation and repair interfaces. An invalid selected graph blocks readiness and claims; inspection/repair and other projects remain available | [[REQ-GRAPH-INTEGRITY]] |
| Repository storage | Per-entity format compatibility, safe initialization, lock/error interfaces, backups and recovery-copy/receipt retention. All authority is in plain files; linked worktrees share coordination, independent clones do not | [[DES-SHARED-FILES]], [[DES-ENTITY-LIFECYCLES]], [[REQ-GIT-CHECKOUT]] |

The confirmed initial scope is recorded in [[ADR-INITIAL-SCOPE]]. [[ADR-RELATION-SEMANTICS]] settles the four relationships, parent completion policies, inherited prerequisites, and lifecycle versus informational distinction. Built-in agent execution, worktree management, and PR/CI actions are outside the initial scope. A TUI remains a later product direction. Multi-machine synchronization, a hosted service, arbitrary workflow scripting, and automatic provider polling have no accepted requirements in this corpus.

The structured external-outcome evaluator proposed in [[REQ-GATE-CONTEXT]] and [[VER-GATE-REVISION]] is retired. External agents interpret review and CI information and add ordinary work and blocking edges; Work does not need a result classifier or revision-aware gate engine. Detailed protocols must preserve that small model.

Snooze/defer scheduling, arbitrary promotion of temporary items, general batch editing, and shared-resource locks are deferred. Template expansion deliberately permits inspectable partial results and caller cleanup/retry. Squash needs bounded retain-before-cleanup ordering; explicit discard intentionally needs no digest. Five fixes followed by consolidation use ordinary dependencies and handoff context; no resource-lock or orchestration primitive is required for that sequence.

[[DES-LIFECYCLE]] settles closure, cancellation, and reopening: cancellation closes an obligation with a reason; reopening recalculates readiness and aggregate state without cascading into completed manual work or restoring discarded operational context. [[DES-ITEM-FORMAT]] settles item serialization, identity, labels, and priority conventions. Explicit discard is a separate supported operation for selected run-owned wisps or a run's ephemeral context, including unfinished work. It does not report success for abandoned obligations or complete material items. A finished run retains its wisps until explicit squash or discard.

## Material risks

:::mara risk RISK-DIVERGENT-VIEWS
:mid: 01M3KAW7AX5F7AF33GXDF9955F
:title: Distinguish selected definitions from shared execution state
:status: accepted
:treatment: open
:affects: REQ-DURABLE-FILES
:affects: DES-SHARED-FILES

Linked worktrees can hold different durable definitions of the same item. A global definition cache keyed only by item ID could mix bodies or relationships from different branches. Keep durable definitions scoped to the selected checkout. Active-run execution state is intentionally shared and takes precedence in normal lifecycle queries; expose its source and distinguish it from the checkout's recorded state. Neither reading shared state nor finalizing a run rewrites or implicitly closes the root. Resolve source-change detection before implementing cross-worktree selection. Obligation: [[REQ-WORKTREE-VIEWS]].
:::

:::mara risk RISK-FILE-DB-CONSISTENCY
:mid: 01M3KAW7B7E9QCWDXAHNGK897V
:title: Interrupted multi-file updates can lose or duplicate work
:status: accepted
:treatment: tolerated
:affects: DES-PERSISTENCE-BOUNDARY
:affects: DES-EPHEMERAL-LAYOUT

Separate entity files have separate atomicity boundaries. A crash during template expansion can leave only some items or relationships. For beta this is an accepted limitation: validate before writing, report known created items on a returned error, leave surviving files inspectable, and let the agent remove partial work and retry. No application record, automatic rollback or idempotency machinery is required. Ordinary invalid-graph diagnostics remain applicable. Other operations must preserve their specific obligations: outgoing context before release and root digest before squash cleanup. Explicit discard intentionally permits deletion without a digest while preserving material work and rejecting unresolved outside references. [[DES-FILE-COORDINATION]], [[REQ-RUN-ATOMICITY]] and [[REQ-RUN-FINALIZATION]] define these boundaries. The stable risk ID is retained from the earlier file/database design.
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
