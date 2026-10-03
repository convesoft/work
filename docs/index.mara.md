# Work product knowledge

Work is a local, agent-first issue tracker implemented in Rust. It lets people plan work and agents select actionable items from a dependency graph. Reusable templates describe repeatable work; temporary execution items keep operational detail out of durable project history.

## Reading this corpus

This corpus captures product intent and proposed contracts. `accepted` means a product direction is established; it does not mean the software exists or a verification has passed. `draft` marks proposals that need a product decision before implementation can treat them as binding.

Mara items describe the Work product. They are not Work's own issue records: Mara IDs, metadata syntax, and acceptance status do not prescribe the runtime item format or lifecycle.

## Corpus map

| Document | Purpose |
| --- | --- |
| [Product](product.mara.md) | Goals, participant roles, and shared vocabulary |
| [Scenarios](scenarios.mara.md) | Planning, actionable work, repeatable review, temporary work, concurrency, and recovery |
| [Requirements](requirements.mara.md) | Observable obligations with accepted versus draft status |
| [Design](design.mara.md) | Persistence boundaries, layouts, lifecycle proposals, and shared interfaces |
| [File storage](storage.mara.md) | Shared plain-file storage, entity locations and lifetimes, locking and recovery boundaries |
| [Item format](item-format.mara.md) | Version-1 item file contract and frontmatter schema |
| [Template format](template-format.mara.md) | Version-1 YAML template and symbolic preview contract; version-2 execution extension in the run API |
| [Execution IO](execution-api.mara.md) | Held-lock IO, whole-file source resolution, authorization and errors |
| [Claims](claims.mara.md) | Immutable acquisitions/endings, scoped selection and atomic claim-next |
| [Runs](runs.mara.md) | Explicit run membership and version-2 mixed template expansion |
| [Execution context](execution-context.mara.md) | Workspaces, named sessions, handoffs and external cleanup reporting |
| [Finalization](run-finalization.mara.md) | Root digests, selected/full discard and retain-before-cleanup |
| [Decisions](decisions.mara.md) | Rationale for the graph, storage model, and initial scope |
| [Verification](verification.mara.md) | Repeatable acceptance checks for delivered and planned slices |
| [Remaining details](open-questions.mara.md) | Engineering contracts, small interface conventions, and material design risks |
| [Delivery](delivery.mara.md) | Ownership of knowledge and tasks, baseline, branches, and release preparation |

## Agreed initial scope

[[ADR-INITIAL-SCOPE]] includes the work graph, reusable templates, temporary runs, multi-agent claims, and both CLI and MCP over shared operations. The TUI is deferred. External tools execute agents, create worktrees, and perform PR/CI actions; Work tracks and coordinates the work.

[[REQ-GIT-CHECKOUT]] requires a Git working checkout. [[REQ-SINGLE-RUN]] keeps one current run per root item and permits a fresh run after finalization/disposal; [[REQ-CLAIM-EXCLUSION]] excludes competing owners across linked worktrees while allowing read-only inspection. The controller can select another worktree by its filesystem path. Normal queries resolve the item's associated worktree and read its actual file, including current body, relationships and state; no run-state overlay is stored. Material updates are saved there immediately and reach main through normal merge. Templates can mix material items and wisps, and material-only planning needs no run; see [[DES-RUN-API]] for the implemented format extension. [[REQ-GRAPH-PROGRESS]] expresses review findings and repeated rounds through ordinary items and blocking edges, without a structured outcome engine.

| State | Authority and location |
| --- | --- |
| Durable work items | Versioned Markdown/YAML under `.work/items/` |
| Reusable templates | Versioned YAML under `.work/templates/` |
| Runs, wisps and workspace references | Separate manifests and entity files under `<Git common directory>/work/runs/`; no template application records |
| Handoff context | Independent Markdown with version-1 frontmatter under `<Git common directory>/work/handoffs/`, retained until all receivers resolve |
| Claims | Immutable acquisition YAML plus a matching ending file under shared `work/claims/` |
| Optional named sessions | Separate YAML files under their run's `sessions/`; one-time sessions need no named registration |
| Reusable workspace associations | Separate YAML files under shared `work/workspaces/`; lifetime independent of an individual claim or run |
| Recovery and operation metadata | Separate records under shared `work/operations/` and retained copies under `work/recovery/` |
| Search and graph views | Disposable in-memory structures derived from authoritative files; no database required |

The durable [bootstrap backlog](../.work/README.md) uses `.work/items/` and [[DES-ITEM-FORMAT]]. It is implementation work, separate from Mara specification items. [[DES-PERSISTENCE-BOUNDARY]] distinguishes project history, shared execution files and shared operational files. [[DES-ENTITY-LIFECYCLES]] gives the complete layout; [[DES-FILE-COORDINATION]] defines the short shared OS lock and recoverable multi-file boundaries. The storage foundation is specified by [[DES-STORE-FOUNDATION]] and [[DES-STORAGE-API]]; execution entity operations use the exact APIs listed below. Durable root digest extensions live in `.work/digests/<root-id>/<run-id>.md`, not the shared run store.

[[DES-WORKSPACE-ASSOCIATIONS]] allows sequential activities and different sessions to reuse a workspace, with a run default and item overrides. An ordinary explicit cleanup item tracks external worktree removal before operational workspace records are removed. Failure retains retry context, and shared unfinished users prevent premature cleanup. Neither claim release nor run completion implicitly deletes a worktree.

## Agreed relationship model

[[DES-RELATION-MODEL]] defines two lifecycle relations (`parent`, `depends_on`) and two informational relations (`related`, `discovered_from`). Children are required work: aggregate parents derive completion from their children, while manual parents execute their own remaining work after children finish. Explicit prerequisites propagate to descendants; internal child waits do not. [[SCN-NESTED-DELIVERY]] illustrates implementation and deployment beneath an automatically completed epic. [[DES-LIFECYCLE]] treats cancellation as closure with a reason and reopening as a readiness recalculation without cascading into completed manual work.

## Provenance

Recent refinements include [[REQ-WORK-SELECTION]] for priority, filtering, and atomic claim-next; [[REQ-GRAPH-INSPECTION]] for graph inspection and readiness explanations; parameterized template preview; and [[REQ-EXECUTOR-HINTS]] for optional model/thinking metadata. [[REQ-OPAQUE-BODIES]] keeps acceptance criteria and other narrative opaque to Work. [[DES-CLAIM-CONTEXT]] records ownership and external session references in shared files. [[DES-HANDOFF-RECORDS]] passes temporary context between one or more source and receiving items without affecting readiness. [[DES-SQUASH-DIGEST]] retains a caller-authored digest extension on the existing root before wisp cleanup, without closing the root. Explicit discard can instead remove selected wisps or dispose of ephemeral run context without a digest, preserving material items and refusing active target claims or unresolved outside references.

This corpus captures product decisions reviewed on 2026-09-28 and revised on 2026-09-30. Explicit user intent establishes the local work tracker, graph and template model, ephemeral operational work, Rust implementation direction, working name `work`, and separate plain files for all authoritative entities. Suggestions remain proposals unless subsequently confirmed.

[[ADR-FILE-STATE]] supersedes the former hybrid SQLite direction. [[ADR-HYBRID-STATE]], [[ADR-SQLITE-COMMON-DIR]] and [[DES-SHARED-SQLITE]] are retained as retired history. The current architecture uses entity-specific folders and files, a short shared OS lock, and explicit recovery protocols. It requires neither custom Git refs nor a socket service. Notifications may refresh a view but cannot establish exclusive ownership.

The shared Git common-directory location and CLI/MCP tracking boundary remain. A single external coordinator normally assigns work, while safe concurrent mutations are still required. Ordinary squash merges retain feature-level project history; operational files are not automatically archived by merging a branch. Retain required results explicitly before cleanup. This revision changes the contract and backlog, not the implemented status below.

## Implementation evidence

The current checkout contains the Cargo workspace, Rust CLI and MCP durable-item operations, read-only template discovery, validation, and symbolic preview, shared file-storage inspection/initialization/recreation/recovery, and automated integration tests. `tests/bootstrap_adoption.rs` exercises durable-item operations on isolated copies of the authored backlog; `tests/templates.rs` and `tests/mcp_protocol.rs` exercise template behavior and CLI/MCP parity. `tests/storage.rs`, `tests/cli_json.rs` and `tests/mcp_protocol.rs` cover the storage foundation, status/warning presentation, and explicit recovery in disposable repositories. [The usage guide](using-work.md) describes the supported commands and current limits. Repository-wide claim acquire/inspect/list/release/recover/reassign and claim-aware material mutations are implemented; `tests/claims.rs` and `tests/coordination.rs` exercise ownership and resolved worktree behavior. File-backed run creation/membership and mixed material/wisp template expansion are implemented; `tests/runs.rs` and `tests/run_execution.rs` cover those boundaries. Receiver-scoped handoff create/inspect/list/receiver replacement/pruning and save-before-close-before-ending are implemented; `tests/handoffs.rs` exercises real CLI/MCP delivery, restart, concurrency, source routing, cancellation, and cross-run retention. `src/core/handoffs/tests.rs` injects publication/completion/pruning failures and checks recovery progress. Reusable workspace management and run-scoped named sessions are implemented; `tests/context_execution.rs` covers reuse, binding repair, named-session ownership context, restart and derived workspace users. Scoped list/readiness and atomic claim-next are implemented; `tests/selection.rs` covers shared filtering, priority/ID ordering, CLI/MCP races and source conflicts. Explicit external workspace cleanup is implemented and covered in `tests/workspace_cleanup.rs`. Run squash and selected/full discard are implemented; `tests/finalization.rs` and operation-specific fault tests cover retention, independent root state and bounded interruption/retry. `tests/beta_adoption.rs` invokes `scripts/verify-beta.mjs` for the connected workflow and all public operations on equivalent disposable linked fixtures; `scripts/smoke-packaged.sh` runs the same harness through installed native/dispatcher packages. The published alpha contains only the durable loop, even though this development checkout still has alpha version metadata. Actual local check logs and artifact/revision hashes, not these descriptions, establish passing evidence on the tested host. A separate release-preparation item owns the generated changelog and publication evidence. Verification definitions describe repeatable checks, not recorded implementation test results; consult actual test runs for passing evidence.

## Execution implementation contracts

The accepted next slices use [shared execution IO](execution-api.mara.md), [claims](claims.mara.md), [runs and template expansion](runs.mara.md), [execution context](execution-context.mara.md), and [squash/discard](run-finalization.mara.md). Claims, scoped selection/claim-next, runs, handoffs, named sessions, workspace management, external workspace cleanup reporting, shared IO/material binding and run squash/discard are implemented in the development checkout. [Worker handoffs](../.work/handoffs/beta-execution.md) preserve historical preparation for the already-integrated claims/runs pair; their dispatch topology and stop point are not current operating instructions. See [delivery conventions](delivery.mara.md) and the selected Work item for current delivery authority.
