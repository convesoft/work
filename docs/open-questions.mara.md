# Remaining engineering details and risks

The product model and initial scope are established. The remaining work below concerns engineering contracts and small interface conventions, not another workflow or orchestration layer. Record concrete decisions in Mara as those contracts are designed; keep unresolved details distinct from accepted behavior.

## Contracts ready for implementation

The beta execution contracts are specified. Accepted knowledge is not implementation or test evidence. The next claims/run handoffs use these exact contracts; workers may choose private Rust helpers without reopening the settled product model.

| Area | Settled contract | Delivery owner |
| --- | --- | --- |
| File IO, errors and transport | [[DES-EXECUTION-IO]] defines IDs, strict envelopes, held-lock integration, resolved item sources, authorization and partial results | Main session coordinates shared core/adapters |
| Claims and selection | [[DES-CLAIM-API]] defines acquisition/ending files, session-pair checks, release/recovery/reassignment and scoped claim-next | Claims item; claim-next remains its dependent item |
| Runs and templates | [[DES-RUN-API]] defines manifests, membership, one current run with later fresh runs, template v2, optional planning root and partial expansion | Runs item |
| Workspaces, sessions and handoffs | [[DES-CONTEXT-API]] defines exact files and commands, source bindings, context retention and external cleanup reporting | Basic source binding in main integration; context/handoff/cleanup items deliver their own operations |
| Squash and discard | [[DES-FINALIZATION-API]] defines per-root/run digest documents, bounded cleanup state, selected/full discard and repeat-call behavior | Finalization item |

No application records, separate material-state overlays, secret ownership tokens, notification daemon, generic transaction engine or additional hardening program are prerequisites. The current storage foundation is reused. Template creation permits partial results and explicit caller cleanup. Claims and run-aware mutations do not treat unavailable storage as healthy.

## Deliberately later work

Backup/restore, automatic pruning/expiry, arbitrary wisp promotion, pagination, remote coordination and the accepted foundation recovery follow-up remain deferred; they do not block these handoffs. Release verification and actual beta version/publication remain the release item's responsibility. CLI command names and JSON fields for the execution slices are now specified; internal Rust decomposition beyond the ownership handoff is an implementation choice.

The implementation dispatch checklist and file ownership are in [the beta handoff](../.work/handoffs/beta-execution.md). The contracts must land on main before fresh implementation worktrees are created from that main. This preparation neither publishes/merges a PR nor authorizes starting a worker.

The confirmed initial scope is recorded in [[ADR-INITIAL-SCOPE]]. [[ADR-RELATION-SEMANTICS]] settles the four relationships, parent completion policies, inherited prerequisites, and lifecycle versus informational distinction. Built-in agent execution, worktree management, and PR/CI actions are outside the initial scope. A TUI remains a later product direction. Multi-machine synchronization, a hosted service, arbitrary workflow scripting, and automatic provider polling have no accepted requirements in this corpus.

The structured external-outcome evaluator proposed in [[REQ-GATE-CONTEXT]] and [[VER-GATE-REVISION]] is retired. External agents interpret review and CI information and add ordinary work and blocking edges; Work does not need a result classifier or revision-aware gate engine. Detailed protocols must preserve that small model.

Snooze/defer scheduling, arbitrary promotion of temporary items, general batch editing, and shared-resource locks are deferred. Template expansion deliberately permits inspectable partial results and caller cleanup/retry. Squash needs bounded retain-before-cleanup ordering; explicit discard intentionally needs no digest. Five fixes followed by consolidation use ordinary dependencies and handoff context; no resource-lock or orchestration primitive is required for that sequence.

[[DES-LIFECYCLE]] settles closure, cancellation, and reopening: cancellation closes an obligation with a reason; reopening recalculates readiness and aggregate state without cascading into completed manual work or restoring discarded operational context. [[DES-ITEM-FORMAT]] settles item serialization, identity, labels, and priority conventions. Explicit discard is a separate supported operation for selected run-owned wisps or a run's ephemeral context, including unfinished work. It does not report success for abandoned obligations or complete material items. A finished run retains its wisps until explicit squash or discard.

## Material risks

:::mara risk RISK-DIVERGENT-VIEWS
:mid: 01M3KAW7AX5F7AF33GXDF9955F
:title: Resolve the correct worktree without mixing item versions
:status: accepted
:treatment: open
:affects: REQ-DURABLE-FILES
:affects: DES-SHARED-FILES

Linked worktrees can hold different versions of the same material item. A global definition cache keyed only by item ID can return stale main content or combine state from one worktree with a body from another. Resolve the item's associated worktree before loading its whole file, and expose that source to the caller. With no association, use the selected checkout. Keep explicit branch-local inspection available; do not treat an unavailable associated worktree as permission to substitute stale content.

Material mutations are saved in that worktree immediately. Run finalization/disposal must neither reset them nor lose the source association needed to find them. Reassociation and external workspace cleanup retain their existing explicit lifecycles. Obligation: [[REQ-WORKTREE-VIEWS]].
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
