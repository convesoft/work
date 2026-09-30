# Decisions and rationale

Decisions separate established direction from proposed details. Alternative sketches in the originating discussion do not imply implemented versions requiring migration.

:::mara decision ADR-GRAPH-TEMPLATES
:mid: 01M3KARNBN27BBZ6NFA43MAQ8S
:title: Represent repeatable process as work items
:status: accepted
:justifies: DES-SHARED-ITEM-MODEL

Decision: use work items and dependency relationships for repeatable activity, including reviews and delivery gates, and generate reusable structures from templates.

Rationale: the user wants agents to pick actionable work rather than repeatedly receive detailed procedural instructions. Keeping temporary and durable activity in the same item model avoids introducing separate user-facing concepts for each workflow stage.

Consequence: templates do not establish a second scheduling semantics. Executor-specific integration and condition refresh still need explicit design.
:::

:::mara decision ADR-HYBRID-STATE
:mid: 01M3KARNBWQ19F4EMBZEMZQYR8
:title: Keep durable files and a side database
:status: retired
:justifies: DES-PERSISTENCE-BOUNDARY

Retired on 2026-09-30; superseded by [[ADR-FILE-STATE]]. The following records the former direction, not a current implementation requirement.

Decision: preserve durable project state in versioned YAML and Markdown, preserve ephemeral work as unversioned files until squash or cleanup, and use SQLite for coordination, maintenance state, and derived indexes.

Rationale: the source discussion established durable files plus a side database. In the corpus-preparation clarification on 2026-09-28, the user preferred ephemeral items in the filesystem rather than solely in SQLite. Inspectable temporary files keep their useful content accessible through the run's lifetime.

Consequences: Git preserves meaningful project intent and results without recording transient updates. The database is partly rebuildable and partly operational. File publication, indexing, and finalization require explicit recovery protocols; SQLite transactions alone do not cover file mutations.
:::

:::mara decision ADR-SQLITE-COMMON-DIR
:mid: 01M3KARNC45Z7KNF0JRFTHNKJC
:title: Select a shared SQLite side database
:status: retired
:justifies: DES-SHARED-SQLITE
:justifies: DES-FILE-LAYOUT
:justifies: DES-EPHEMERAL-LAYOUT

Retired on 2026-09-30; superseded by [[ADR-FILE-STATE]]. The following records the former direction, not a current implementation requirement.

Decision: store durable items and templates under `.work/`, use SQLite at `<Git common directory>/work/work.db`, and retain authoritative ephemeral work files under `<Git common directory>/work/runs/`. Linked worktrees share operational storage; durable files retain their checkout-specific Git versions.

Rationale: during corpus preparation on 2026-09-28, the user accepted the hybrid storage direction, preferred temporary work on the filesystem until squash or cleanup, and explicitly selected the shared Git common directory for those files. The shared location supports coordination across isolated code checkouts without versioning operational detail.

Trade-offs: this does not synchronize separate clones. Branch-local durable views require explicit indexing semantics, and file-backed mutations need coordination and reconciliation with SQLite. No illustrative table schema or SQL from the source conversation is adopted.
:::

:::mara decision ADR-INITIAL-SCOPE
:mid: 01M3KATNEJXDNDPF6VA3JTVZTH
:title: Deliver coordination through CLI and MCP first
:status: accepted
:justifies: DES-INTERFACE-ADAPTERS

Decision: the initial release includes CLI and MCP, the work graph, templates, file-backed temporary runs, and multi-agent claims. Defer the TUI. Keep agent execution, worktree management, and PR/CI actions in external tools.

Rationale: in the corpus-preparation clarification on 2026-09-28, the user selected the coordination-first initial scope and added MCP because CLI and MCP are different surfaces for the same commands.

Consequences: shared core operations and equivalent error semantics are required from the first release. This scope does not decide every lifecycle, serialization, or recovery detail; draft contracts and open questions still require resolution before implementing affected behavior.

Subsequent accepted refinements include priority and scoped selection, atomic claim-next, readiness explanations, graph inspection, parameterized template preview, optional model/thinking hints, file-backed handoffs between items, and caller-authored squash digests. Keep Markdown bodies opaque outside initial template rendering. Defer snooze/defer scheduling, arbitrary temporary-item promotion, general batch editing, and shared-resource locks. Narrow multi-file template publication and squash still require recoverable protocols despite deferring a general batch API.
:::

:::mara decision ADR-RELATION-SEMANTICS
:mid: 01M3KFAJ6FRTZ6AA1QWKY0X432
:title: Use two lifecycle and two informational relations
:status: accepted
:justifies: DES-RELATION-MODEL

Decision: adopt parent and depends_on as lifecycle relations, and related and discovered_from as informational relations. Children represent required decomposition. Aggregate parents complete from their children; manual parents retain an explicit final execution step after their children finish.

Rationale: hierarchy must express the complete scope of a deliverable, so depending on that deliverable waits for its full outcome. Discovery provenance separately records follow-up work without silently expanding scope or blocking delivery. The user accepted this model and the nested implementation/deployment epic example.

Consequences: readiness must distinguish inherited explicit prerequisites from internal child waits, and graph validation must detect combined lifecycle deadlocks. Sibling ordering remains explicit. Closure with a cancellation reason resolves an obligation. Reopening recomputes readiness and aggregate completion without automatically reopening completed manual work, as specified in [[DES-LIFECYCLE]].
:::

:::mara decision ADR-WORK-DISTRIBUTION
:mid: 01M3M7C6321TM1WZ0JEG2N54AF
:title: Select initial Work package identities and hosts
:status: accepted
:justifies: DES-INTERFACE-ADAPTERS

Use @convesoft/work as the npm dispatcher and @convesoft/work-linux-x64-gnu, @convesoft/work-linux-arm64-gnu, and @convesoft/work-darwin-arm64 as native packages. Support x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, and aarch64-apple-darwin, matching Mara’s supported target platforms. The first candidate is 0.1.0-alpha.1 and is dual-licensed under MIT OR Apache-2.0. npm trusted publishing identifies the convesoft/work GitHub repository and release.yml workflow without an npm environment-name restriction. GitHub release-environment protection is a separate required setting. Registry trust configuration and GitHub environment protection require verification as actual server-side settings before publication.
:::

:::mara decision ADR-PETGRAPH
:mid: 01M3MCB75WXQ4DMMVAE4B2AJQ9
:title: Use petgraph for internal graph algorithms
:status: accepted
:justifies: DES-RELATION-MODEL
:justifies: DES-LIFECYCLE

Decision: use the Rust petgraph library for Work's internal graph representation, traversal, and cycle detection, following the same library choice as Mara. Start from Mara's dependency configuration: `petgraph = { version = "0.8.3", default-features = false, features = ["std"] }`. This is the initial dependency baseline, not a permanent version or public-format constraint; Cargo.lock records the resolved version.

Rationale: reuse established graph structures and algorithms rather than implementing generic traversal and cycle detection ourselves. Consistency with Mara provides a familiar implementation reference without making Work depend on Mara's product-specific graph model.

Boundaries: keep canonical Work item IDs separate from petgraph node and edge indices. Those indices are internal, disposable implementation details and must not become persisted identities or CLI/MCP identifiers. Authoritative item files and their relationships remain governed by the existing storage and item-format contracts.

Work retains its own evaluator for readiness, inherited prerequisites, manual-parent execution, aggregate completion, and explanations. Construct lifecycle checks from those semantics; do not treat every stored relationship as an ordinary blocking edge. Informational `related` and `discovered_from` relationships remain available for navigation but are excluded from lifecycle cycle/deadlock checks. Generic library algorithms alone do not establish correct Work scheduling.

Consequences: petgraph supplies graph mechanics, while Work owns semantic-edge normalization and deterministic user-visible ordering. The exact graph type and in-memory layout remain implementation choices. Recording this decision does not add the dependency or claim that graph behavior is implemented.
:::

:::mara decision ADR-FILE-STATE
:mid: 01M3RPEMJZNGNSY8GSDZRNM3JK
:title: Store all Work state in separate plain files
:status: accepted
:supersedes: ADR-HYBRID-STATE
:supersedes: ADR-SQLITE-COMMON-DIR
:justifies: DES-PERSISTENCE-BOUNDARY
:justifies: DES-SHARED-FILES
:justifies: DES-ENTITY-LIFECYCLES
:justifies: DES-FILE-COORDINATION

Decision, revised 2026-09-30: store every authoritative entity in Markdown/YAML, organized by its operational or execution lifetime. Keep project intent and retained feature results in ordinary Git-tracked files; keep live claims, runs, sessions, workspace context, handoffs and recovery records in shared plain files. Use the existing Git common-directory discovery and a short OS file lock for concurrent mutations. Do not introduce SQLite, custom Git refs, a monolithic entity snapshot or a required socket service.

Rationale: the expected workflow has one external coordinator assigning work, with modest local concurrency. Separate inspectable entity files fit that workflow and preserve useful context without database tooling. Feature-level history through ordinary squash merges is sufficient; each claim transition need not become a commit. A shared location survives checkout changes and removal while Work exposes another checkout's requested context through CLI/MCP.

Trade-offs: Work now owns schema validation, source-conflict detection, lock discipline, crash-safe replacement and narrow multi-file recovery. Plain files do not make notifications reliable exclusion or make several writes atomic. Queries may parse more files than an indexed database; start with disposable in-memory views rather than an unproven persistent cache. Independent clones and multi-host coordination remain outside scope. Git backs up committed durable content, not live shared execution state; backup and explicit recovery remain separate responsibilities.

This supersedes the SQLite direction recorded on 2026-09-28. The retired decisions and design retain their original IDs and MIDs as history. This acceptance revises the architecture and delivery scope; it does not establish that the paused SQLite implementation or the replacement file coordination has shipped.

Foundation scope clarification on 2026-09-30: defer backup creation and restoration. Deliver safe entity-file primitives, shared locking, validation and diagnostics first, preserving the rule to report missing/damaged state and require explicit action before recreation. Entity-aware backup/restore can be specified after the relevant formats exist; it is not an acceptance gate for the initial storage foundation.
:::
