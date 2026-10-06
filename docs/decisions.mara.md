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
:justifies: RISK-FILE-DB-CONSISTENCY

Decision: the initial release includes CLI and MCP, the work graph, templates, file-backed temporary runs, and multi-agent claims. Defer the TUI. Keep agent execution, worktree management, and PR/CI actions in external tools.

Rationale: in the corpus-preparation clarification on 2026-09-28, the user selected the coordination-first initial scope and added MCP because CLI and MCP are different surfaces for the same commands.

Consequences: shared core operations and equivalent error semantics are required from the first release. This scope does not decide every lifecycle, serialization, or recovery detail; draft contracts and open questions still require resolution before implementing affected behavior.

Subsequent accepted refinements include priority and scoped selection, atomic claim-next, readiness explanations, graph inspection, parameterized template preview, optional model/thinking hints, file-backed handoffs between items, and caller-authored squash digests. Keep Markdown bodies opaque outside initial template rendering. Defer snooze/defer scheduling, arbitrary temporary-item promotion, general batch editing, and shared-resource locks. Template expansion uses ordinary per-file creation with explicit partial-result reporting and caller cleanup/retry, without application records or all-or-nothing publication. Finalization retains a digest extension to the root before temporary cleanup; it does not create a digest item or close the root.
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
:status: retired
:supersedes: ADR-HYBRID-STATE
:supersedes: ADR-SQLITE-COMMON-DIR
:justifies: DES-PERSISTENCE-BOUNDARY
:justifies: DES-SHARED-FILES
:justifies: DES-ENTITY-LIFECYCLES
:justifies: DES-FILE-COORDINATION

Retired as a beta.2 contract; retained as historical beta.1 knowledge, not an obligation for the replacement implementation. See [[ADR-SCOPE-OWNED-EXECUTION]]. The executable may still implement this historical behavior until the beta.2 delivery items are merged.

Decision, revised 2026-09-30: store every authoritative entity in Markdown/YAML, organized by its operational or execution lifetime. Keep project intent and retained feature results in ordinary Git-tracked files; keep live claims, runs, sessions, workspace context, handoffs and recovery records in shared plain files. Use the existing Git common-directory discovery and a short OS file lock for concurrent mutations. Do not introduce SQLite, custom Git refs, a monolithic entity snapshot or a required socket service.

Rationale: the expected workflow has one external coordinator assigning work, with modest local concurrency. Separate inspectable entity files fit that workflow and preserve useful context without database tooling. Feature-level history through ordinary squash merges is sufficient; each claim transition need not become a commit. A shared location survives checkout changes and removal while Work exposes another checkout's requested context through CLI/MCP.

Trade-offs: Work now owns schema validation, source-conflict detection, lock discipline, crash-safe replacement and narrow multi-file recovery. Plain files do not make notifications reliable exclusion or make several writes atomic. Queries may parse more files than an indexed database; start with disposable in-memory views rather than an unproven persistent cache. Independent clones and multi-host coordination remain outside scope. Git backs up committed durable content, not live shared execution state; backup and explicit recovery remain separate responsibilities.

This supersedes the SQLite direction recorded on 2026-09-28. The retired decisions and design retain their original IDs and MIDs as history. This acceptance revises the architecture and delivery scope; it does not establish that the paused SQLite implementation or the replacement file coordination has shipped.

Foundation scope clarification on 2026-09-30: defer backup creation and restoration. Deliver safe entity-file primitives, shared locking, validation and diagnostics first, preserving the rule to report missing/damaged state and require explicit action before recreation. Entity-aware backup/restore can be specified after the relevant formats exist; it is not an acceptance gate for the initial storage foundation.
:::

:::mara decision ADR-SCOPE-OWNED-EXECUTION
:mid: 01M439Z993Q5A39J69425G52QM
:title: Replace global execution bookkeeping with stable scope ownership
:status: accepted
:supersedes: ADR-FILE-STATE
:justifies: DES-SCOPE-RESOLUTION
:justifies: DES-SCOPE-LAYOUT
:justifies: DES-SCOPED-RUNS
:justifies: DES-IMMUTABLE-SESSIONS
:justifies: DES-SCOPED-CLAIMS
:justifies: DES-SENDER-HANDOFFS
:justifies: DES-SCOPE-TRANSITIONS
:justifies: DES-BETA2-SURFACES
:justifies: RISK-FILE-DB-CONSISTENCY

Decision for planned 0.1.0-beta.2: use an implicit item-rooted scope tree beneath a virtual repository root hosted by Git's primary checkout. Store anchors only at explicit run/hosting boundaries. Scopes own stable identity and parent/child routing; Git supplies checkout facts; per-checkout Git-local declarations establish one authoritative host. Runs own execution rounds, temporary work, immutable named sessions, claims and sender-local addressed handoffs, with one storage home. Descendants inherit the nearest run until overriding it.

Rationale: local placement does not prevent cross-scope access through a resolver. Independent global lifetimes, workspace registries, explicit membership and handoff retention obligations duplicated ownership facts. Versionable scope content should be able to commit/merge and continue in a parent checkout without resetting run/session/claim identity. Destination metadata is not a retention obligation. Parent orchestrators decide promotion, optional result retention and explicit disposal.

Consequences: initialize a standing default root run; resolve readable session shortcuts and drain immutable sessions; derive context/membership; use pull discovery and optional finalization summaries; dispose explicit subtrees bottom-up without handoff archival gates. Keep repository-wide claim exclusion, explicit authoritative relocation and executor-quiescence safety, graph integrity, source diagnostics, partial-result reporting and CLI/MCP parity. Git/worktree actions and agent/process execution stay external. No beta.1 execution migration or dual-layout compatibility is required. Remove superseded commands; exact wire definitions are deliberately draft until frozen.

This supersedes the shared placement/lifetime choices in ADR-FILE-STATE, not the enduring plain-file, opaque-body, graph and local-coordination principles. Historical items retain IDs/MIDs and bodies; accepted beta.2 knowledge is neither implemented behavior nor publication evidence.

## Scope-review clarifications

Confirmed product choices: ended child execution keeps its boundary until explicit new execution or resumption of inheritance; retained wisps remain historical in their original run, with inspection/promotion/disposal but no ordinary edits, reopening, new claims or new handoffs. Root rotation requires explicit abandonment of unfinished root wisps and does not require completion of the material repository backlog. Ending automatically retires idle named identities after their claims across all runs drain.

Execution disposal preserves material hosting until separately validated relocation. Physical-copy inspection is read-only; Work writes through authoritative routing. Child-run templates declare parentage explicitly rather than having Work invent edges or a second membership rule. Item-addressed context follows stable item identity; run/session addresses remain immutable. Unqualified named claim-next selects within one resolved run namespace instead of letting a candidate choose which external identity a name means.

Rationale: these choices avoid silent changes in execution namespace, ownership, durable source or lifecycle graph while retaining explicit recovery of useful temporary work. Product questions in this review are settled; exact file fields, command spellings, tool schemas and bounded continuation representations remain the separate DES-BETA2-WIRE engineering gate, not implementation or release evidence.

The same explicit-placement rule applies after creation: current-wisp reparenting into a different effective run is rejected. Same-run reparenting remains ordinary graph work; promote first when cross-run movement is needed. Beta.2 does not add wisp-transfer machinery.

Wire-review clarification: the user selected material-only roots for stored run/dedicated-host boundaries (promote a wisp first). The user also selected source recovery before adoption when old authoritative bytes are unavailable; attestation alone cannot verify continuing ownership. These choices preserve one-home temporary lifetimes and avoid treating an unverifiable copy as current state. Exact formats and selectors are specified by DES-BETA2-WIRE; no runtime or release evidence is implied.


## Supported Git layouts

The user explicitly narrowed beta.2 to ordinary repositories with a primary .git directory and their linked worktrees, including detached primary checkouts. git init --separate-git-dir is unsupported. A fresh separate Git directory need not retain a primary working-checkout backpointer; resolving it from a linked checkout would require an additional locator or discovery policy. This uncommon layout is deferred for unscheduled investigation, not a current hosting acceptance requirement. Reject it without writes instead of guessing, configuring Git, or adding a persistent registry.
:::

:::mara decision ADR-RECOVERY-ARTIFACT-LIFECYCLE
:mid: 01M443ZBYD2M12DMRJDEJV1HNB
:title: Use temporary staging across all Work-managed file publication
:status: accepted
:justifies: DES-DURABLE-OPERATIONS
:justifies: DES-FILE-PUBLICATION-LIFECYCLE

Decision for planned `0.1.0-beta.2`: adopt Mara's temporary-staging lifecycle across **all Work-managed file publication**, rather than preserving a previous-version file after every successful replacement. Apply the same policy to project content, scope-owned execution, Git-local routing/coordination and operation-specific progress. Keep recovery information for unresolved publication, detected conflicts and displaced repair sources; remove only obsolete artifacts after verified ordinary success or explicit resolution. [[DES-FILE-PUBLICATION-LIFECYCLE]] is the common contract; [[DES-DURABLE-OPERATIONS]] retains item-specific publication/repair behavior.

Rationale: retaining one copy per successful mutation makes storage grow with the number of writes, even when nothing failed. Fixing only the item writer leaves the shared publisher and non-item record updates accumulating `.storage-*` or equivalent copies. Artifacts support a concrete operation, not unlimited edit history. Retention by age or copy count does not establish safe deletion.

Trade-off: preserve detected conflicts and actionable publication evidence, but do not guarantee preservation of later writes through stale handles held by outside editors. Callers must avoid concurrent direct editing and Work mutations of the same source. Work locks do not coordinate those editors or external Git operations.

The scope is obsolete publication artifacts, not authoritative records: immutable claims, retained run/session/handoff/digest content, live locks and still-needed continuation evidence keep their own lifecycles. Preserve descriptor-bound access, permissions, sync, ownership, graph safeguards and bounded continuation. Do not add a generic journal/rollback service, new editing APIs, beta.1 migration or automatic deletion of existing artifacts. This is accepted future policy, not implemented behavior or passing execution evidence.
:::
