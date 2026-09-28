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
| [Item format](item-format.mara.md) | Version-1 item file contract and frontmatter schema |
| [Decisions](decisions.mara.md) | Rationale for the graph, storage model, and initial scope |
| [Verification](verification.mara.md) | Repeatable future acceptance checks |
| [Remaining details](open-questions.mara.md) | Engineering contracts, small interface conventions, and material design risks |
| [Delivery](delivery.mara.md) | Ownership of knowledge and tasks, baseline, branches, and release preparation |

## Agreed initial scope

[[ADR-INITIAL-SCOPE]] includes the work graph, reusable templates, temporary runs, multi-agent claims, and both CLI and MCP over shared operations. The TUI is deferred. External tools execute agents, create worktrees, and perform PR/CI actions; Work tracks and coordinates the work.

[[REQ-GIT-CHECKOUT]] requires a Git working checkout. [[REQ-SINGLE-RUN]] keeps one execution run per root item; [[REQ-CLAIM-EXCLUSION]] excludes competing owners across linked worktrees while allowing read-only inspection. The controller can select another worktree by its filesystem path. [[REQ-GRAPH-PROGRESS]] expresses review findings and repeated rounds through ordinary items and blocking edges, without a structured outcome engine.

| State | Authority and location |
| --- | --- |
| Durable work items | Versioned Markdown/YAML under `.work/items/` |
| Reusable templates | Versioned YAML under `.work/templates/` |
| Ephemeral work content and relationships | Unversioned files under `<Git common directory>/work/runs/`, retained until squash or cleanup |
| Handoff context | Unversioned Markdown under `<Git common directory>/work/handoffs/`, retained until all receiving items finish |
| Claims, observations, and other coordination records | SQLite at `<Git common directory>/work/work.db` |
| Optional named sessions and reusable workspace associations | Shared SQLite; names are scoped to a run, while one-time sessions need no named registration |
| Search and graph indexes | Derived SQLite data, rebuildable from the authoritative files |

The durable [bootstrap backlog](../.work/README.md) now uses `.work/items/` and [[DES-ITEM-FORMAT]]. It is real implementation work, separate from Mara specification items. No coordination database or run state has been initialized. [[DES-PERSISTENCE-BOUNDARY]] distinguishes reconstructible file-derived data from database-only coordination state.

[[DES-WORKSPACE-ASSOCIATIONS]] allows sequential activities and different sessions to reuse a workspace, with a run default and item overrides. An ordinary explicit cleanup item tracks external worktree removal before operational workspace records are removed. Failure retains retry context, and shared unfinished users prevent premature cleanup. Neither claim release nor run completion implicitly deletes a worktree.

## Agreed relationship model

[[DES-RELATION-MODEL]] defines two lifecycle relations (`parent`, `depends_on`) and two informational relations (`related`, `discovered_from`). Children are required work: aggregate parents derive completion from their children, while manual parents execute their own remaining work after children finish. Explicit prerequisites propagate to descendants; internal child waits do not. [[SCN-NESTED-DELIVERY]] illustrates implementation and deployment beneath an automatically completed epic. [[DES-LIFECYCLE]] treats cancellation as closure with a reason and reopening as a readiness recalculation without cascading into completed manual work.

## Provenance

Recent refinements include [[REQ-WORK-SELECTION]] for priority, filtering, and atomic claim-next; [[REQ-GRAPH-INSPECTION]] for graph inspection and readiness explanations; parameterized template preview; and [[REQ-EXECUTOR-HINTS]] for optional model/thinking metadata. [[REQ-OPAQUE-BODIES]] keeps acceptance criteria and other narrative opaque to Work. [[DES-CLAIM-CONTEXT]] records session and workspace associations in SQLite. [[DES-HANDOFF-RECORDS]] passes temporary context between one or more source and receiving items without affecting readiness. [[DES-SQUASH-DIGEST]] preserves a caller-authored summary as an ordinary completed durable item before temporary cleanup.

This corpus captures product discussions and decisions reviewed on 2026-09-28. Explicit user intent establishes the local work tracker, graph and template model, ephemeral operational work, Rust implementation direction, working name `work`, and durable YAML/Markdown with a side database. Suggestions from the assistant are proposals unless subsequently confirmed.

The discussion evolved from evaluating existing products through SQLite-only and filesystem-only sketches to hybrid persistence. Those earlier sketches are background alternatives, not simultaneous requirements. Commands, Rust structs, database tables, library choices, and version numbers shown there were illustrative; this corpus does not adopt them as implemented interfaces. No current claims about competing products are carried over.

The current corpus-preparation conversation on 2026-09-28 clarified three decisions: retain ephemeral work as files until squash or cleanup, place those files in the shared Git common directory, and include MCP with CLI in an initial release limited to tracking and coordination. These clarifications take precedence over earlier suggestions to store ephemeral items only in SQLite or defer MCP.

## Implementation evidence

The repository baseline contains agent instructions, Mara knowledge, a version-1 item schema, and implementation item files, but no Cargo manifest or Rust implementation. The first feature branch establishes the Rust foundation and initial CI/release workflows. A separate release-preparation item follows the first usable CLI/MCP milestone and owns the generated changelog and publication evidence. Verification definitions describe future checks, not recorded implementation test results.
