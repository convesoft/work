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
:status: accepted
:justifies: DES-PERSISTENCE-BOUNDARY

Decision: preserve durable project state in versioned YAML and Markdown, preserve ephemeral work as unversioned files until squash or cleanup, and use SQLite for coordination, maintenance state, and derived indexes.

Rationale: the source discussion established durable files plus a side database. In the corpus-preparation clarification on 2026-09-28, the user preferred ephemeral items in the filesystem rather than solely in SQLite. Inspectable temporary files keep their useful content accessible through the run's lifetime.

Consequences: Git preserves meaningful project intent and results without recording transient updates. The database is partly rebuildable and partly operational. File publication, indexing, and finalization require explicit recovery protocols; SQLite transactions alone do not cover file mutations.
:::

:::mara decision ADR-SQLITE-COMMON-DIR
:mid: 01M3KARNC45Z7KNF0JRFTHNKJC
:title: Select a shared SQLite side database
:status: accepted
:justifies: DES-SHARED-SQLITE
:justifies: DES-FILE-LAYOUT
:justifies: DES-EPHEMERAL-LAYOUT

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
