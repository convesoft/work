# Architecture and data contracts

Contracts here describe the intended product architecture. Exact command spelling, serialization fields, and Rust crate layout remain uncommitted unless explicitly specified. [File storage](storage.mara.md) defines the current shared entity layout and coordination boundaries.

:::mara design DES-SHARED-ITEM-MODEL
:mid: 01M3KARN9Y8H41N8FF61NVHZZ7
:title: Use one work-item graph model
:status: accepted
:kind: structure
:satisfies: REQ-WORK-GRAPH
:satisfies: REQ-GRAPH-TEMPLATES
:satisfies: REQ-EPHEMERAL-WORK

Work items are graph nodes with relationships expressing process constraints. A template is a reusable graph definition; instantiation yields ordinary nodes and edges. Durable and ephemeral items share graph semantics. Review and delivery gates fit the same model; how a node's outcome is supplied may differ. [[DES-RELATION-MODEL]] defines the accepted four-relation vocabulary and parent completion policies. Internal graph representations and executor configuration remain to be designed.
:::

:::mara design DES-PERSISTENCE-BOUNDARY
:mid: 01M3KARNA567E2TXS0XP40EYQB
:title: Separate project history from operational and execution files
:status: accepted
:kind: data
:satisfies: REQ-DURABLE-FILES
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-EPHEMERAL-FILES

All authoritative Work state lives in separate Markdown/YAML entity files:

- Versioned project files own durable intent, relationships and retained feature outcomes.
- Shared unversioned execution files own run manifests, wisps, active-run execution state and handoff context.
- Shared unversioned operational files own claims, workspace associations, optional named sessions and recoverable operation metadata.

[[DES-SHARED-FILES]] defines location and checkout isolation; [[DES-ENTITY-LIFECYCLES]] defines each entity's place and retention. There is no database or single repository-wide entity snapshot. In-memory graph/lookup data is disposable and derived from these files. Process restart does not discard operational ownership. Rebuilding a derived view never resets claims or erases execution context.

Ordinary commits and squash merges retain feature-level history. Live operational state is not committed merely because it changes, and a branch merge does not archive it automatically. Retain the required outcomes in durable item bodies or a caller-authored digest before cleanup. Temporary means bounded by an explicit lifecycle, not disposable on process exit.
:::

:::mara design DES-FILE-LAYOUT
:mid: 01M3KARNADPDJ6FTA3TKGYRPNX
:title: Store per-item documents and YAML templates under .work
:status: accepted
:kind: data
:satisfies: REQ-DURABLE-FILES

Use `.work/items/` for durable Markdown item documents with YAML frontmatter and `.work/templates/` for versioned YAML template definitions. Keep one document per durable item. Long descriptions, acceptance criteria, and retained outcomes live in Markdown; machine-readable metadata and relationships live in frontmatter.

Use an immutable random UUIDv4 encoded as 32 lowercase hexadecimal characters as the canonical item identity and filename stem (`<id>.md`). Store full IDs in relationships. Human output can display `w-` followed by a short prefix, initially eight hexadecimal characters, extended as necessary to disambiguate. Resolve a shortened input only when unambiguous; never silently choose among matches. Detect full-ID collisions rather than overwriting existing work. Do not introduce sequential aliases or encode hierarchy in IDs. [[DES-ITEM-FORMAT]] defines the version-1 header fields, aggregate state omission, relationship serialization, unknown-field rejection, and byte-preserving body framing. These are Work files, not Mara documents. Ephemeral item files use the shared unversioned location in [[DES-EPHEMERAL-LAYOUT]].

Markdown body preservation follows [[REQ-OPAQUE-BODIES]]: acceptance criteria stay in the body; Work does not parse or update their sections. Optional `model` and `thinking` metadata follow [[REQ-EXECUTOR-HINTS]].
:::

:::mara design DES-SHARED-SQLITE
:mid: 01M3KARNAN45B1D7CSCEKHHTSK
:title: Share SQLite through the Git common directory
:status: retired
:kind: deployment
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-CLAIM-EXCLUSION
:satisfies: REQ-WORKTREE-VIEWS
:satisfies: REQ-INDEX-REBUILD

Retired on 2026-09-30; superseded by [[DES-SHARED-FILES]]. The following records the former direction, not a current implementation requirement.

Use SQLite at `<resolved Git common directory>/work/work.db` for repository-local coordination, maintenance state, and derived indexes. Discover the common directory from the selected worktree instead of assuming `.git` is a directory in that checkout. Linked worktrees share the store; independent clones do not automatically share it.

Durable views need explicit worktree/source-version scoping, including uncommitted changes; a single global row keyed only by item ID cannot represent divergent versions safely. Claims and observations belong in SQLite, while ephemeral item content and relationships remain authoritative in shared files under [[DES-EPHEMERAL-LAYOUT]]. Claims are exclusive per item across this Git repository and its linked worktrees; source views do not create separate execution identities. Queries can select another worktree directly by filesystem path. File/index reconciliation still needs its detailed contract.
:::

:::mara design DES-LIFECYCLE
:mid: 01M3KARNAXWP2BJCW5B4RZ8YM2
:title: Separate recorded outcomes from derived activity
:status: accepted
:kind: behavior
:satisfies: REQ-ACTIONABLE-WORK
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-CLAIM-RECOVERY

Use two lifecycle states: open and done. Closing an item records done and may retain a caller-supplied closure reason. Cancellation is explicit closure with a reason, not a separate scheduling state: it resolves that obligation for parents and dependents. Work does not parse the reason or infer whether deliverables exist. The caller adjusts required work and relationships before closing an obligation that has been replaced or removed.

Readiness and blocking derive from lifecycle prerequisites; ownership derives from claims rather than durable in-progress edits. [[DES-RELATION-MODEL]] defines manual and aggregate completion. Reopening a retained item makes it open and recomputes readiness and aggregate completion, but does not automatically reopen completed manual parents or dependents. Reopening restores neither deleted handoffs nor removed workspaces. If a temporary item was removed by squash, subsequent work uses a new item. Finishing implementation does not itself finish other required activities.
:::

:::mara design DES-TEMPLATE-RUNS
:mid: 01M3KARNB5PY2J48QMVKYKMBSM
:title: Group instantiated work by run
:status: accepted
:kind: structure
:satisfies: REQ-GRAPH-TEMPLATES
:satisfies: REQ-RUN-ATOMICITY
:satisfies: REQ-RUN-FINALIZATION

A template creates predefined items and relationships. Each item is material (durable) or a wisp (temporary), and one template can mix both. There is no planning/execution category on the template. Material items go to the selected checkout's `.work/items/`; wisps go to a target run's `items/`. A material-only expansion needs no run. A template containing wisps needs a target run; exact CLI/MCP selection and the per-item format extension must be specified before implementation.

An item has one run; callers cannot create competing independent executions for it. A run groups the root's execution, wisps and active execution state. Run membership is operational grouping, not an implicit parent relationship or an extra graph task. Normal queries use the active-run state as specified by [[REQ-WORKTREE-VIEWS]]. Sessions and workspaces may span items. External controllers choose dispatch and parallelism; Work does not launch agents.

Repeated expansion can add findings, fixes and new review rounds through ordinary items and blocking edges. It creates no application record, provenance entity or idempotency receipt. Preview uses local keys; creation assigns permanent IDs and returns their mapping as an operation result. Validate the prospective graph before writing; if writing stops partway, report known created items and leave inspection, deletion and retry to the caller under [[REQ-RUN-ATOMICITY]]. No atomic multi-file publication or automatic retry deduplication is promised. Here, publication means creating item files, not creating an additional lifecycle entity.

The run has finished when its member obligations are resolved and no active claims remain. The root is separate: an unfinished manual root does not prevent retaining a finished run's digest, and finalization does not complete it. [[DES-SQUASH-DIGEST]] retains a caller-written extension to the root before wisp cleanup. Durable output uses a designated surviving checkout and is not automatically committed or merged. Shared files survive feature-worktree removal. Workspace cleanup is separate and external; run completion never removes a shared workspace. Refuse cleanup that would strand outside references.
:::

:::mara design DES-INTERFACE-ADAPTERS
:mid: 01M3KARNBDCQ15K8BRB44P8276
:title: Keep interfaces over shared Rust domain behavior
:status: accepted
:kind: structure
:satisfies: REQ-LOCAL-STATE
:satisfies: REQ-CLI-JSON
:satisfies: REQ-CLI-MCP-PARITY
:satisfies: REQ-EXTERNAL-EXECUTION

Implement Work in Rust with a shared operation layer for items, graph queries and mutations, templates, runs, and coordination. CLI and MCP are both initial-release surfaces over the same operation semantics; transport-specific formatting must not create different business rules. The human TUI is deferred.

Work tracks and coordinates work; external tools run agents, create worktrees, and perform PR/CI actions. It can record supplied context and observations without executing those external activities. This contract does not prescribe separate crates, particular Rust libraries, a daemon, command/tool names, or response schemas.
:::

:::mara design DES-EPHEMERAL-LAYOUT
:mid: 01M3KAW78VAFH89JHJHREXRKP9
:title: Share temporary work files across linked worktrees
:status: accepted
:kind: data
:satisfies: REQ-EPHEMERAL-FILES
:satisfies: REQ-SIDE-STATE

Store each run under `<resolved Git common directory>/work/runs/<run-id>/`, with a `run.yaml` manifest, separate `items/` wisp files, and optional `sessions/` records. No `applications/` directory or template application entity is required. Linked worktrees share this location; independent clones do not.

The manifest owns run/root identity and source/output checkout context. Wisps own their metadata, relationships and bodies. Active-run state for durable items must have an explicit file representation without copying durable bodies or embedding all entities into the manifest; the exact envelope belongs to the run implementation. Handoffs, claims and workspace records remain independent entities with their own lifecycles.

[[DES-FILE-COORDINATION]] defines locking and individual file safety. Template expansion may leave a partial result for caller inspection and cleanup under [[REQ-RUN-ATOMICITY]]; it requires no committed-manifest visibility protocol. Retain required results as a root digest extension before explicitly removing temporary context.
:::

:::mara design DES-RELATION-MODEL
:mid: 01M3KFAJ52WE9D3QJMVJVKC8S6
:title: Separate lifecycle and informational relationships
:status: accepted
:kind: behavior
:satisfies: REQ-PARENT-COMPLETION
:satisfies: REQ-HIERARCHICAL-READINESS
:satisfies: REQ-INFORMATIONAL-RELATIONS
:satisfies: REQ-RELATION-STRUCTURE

Work has four item relationships:

| Relation | Category | Meaning and direction |
| --- | --- | --- |
| `parent` | Lifecycle | Child points to its parent; children contribute required work to parent completion |
| `depends_on` | Lifecycle | Dependent points to the prerequisite whose complete outcome it requires |
| `related` | Informational | Symmetric contextual association |
| `discovered_from` | Informational | Discovered item points to the item being worked on when it was discovered |

Store each semantic edge once. Derive `children` from `parent` and `blocks` from `depends_on`; expose `related` symmetrically. An item has at most one parent. Informational edges can coexist with lifecycle edges and never change scheduling. These are Work product relations, separate from the Mara schema's knowledge relations.

`completion: manual` is the default. A manual parent becomes executable only after its children finish, then requires explicit completion of its own remaining work. `completion: children` identifies a non-executable aggregate: it cannot be claimed and derives done from a nonempty set of completed direct children. Aggregation composes recursively and does not require automatic parent-file rewrites. Empty aggregates remain open; adding or reopening unfinished children makes an aggregate incomplete again.

Explicit prerequisites on a parent gate its descendants. A parent's internal wait for children does not gate those children. Siblings can run concurrently unless explicit dependencies order them. Depending on a parent waits for children plus any manual parent execution. Reject cycles and combined lifecycle deadlocks; do not translate every parent-child link into an ordinary blocking edge.

For preparation that must precede other children, create a preparation child and explicit dependencies. A manual parent then represents final integration or acceptance. [[SCN-NESTED-DELIVERY]] provides the nested epic example.

Closing with a cancellation reason resolves the obligation just like other closure. Reopening recomputes readiness and aggregates but does not cascade reopening into completed manual parents or dependents; see [[DES-LIFECYCLE]]. A separate waits-for relation is deferred.
:::

:::mara design DES-HANDOFF-RECORDS
:mid: 01M3KPWZ42EZ05TAB0G8GNHCBJ
:title: Store transition context in shared temporary files
:status: accepted
:kind: data
:satisfies: REQ-HANDOFF-CONTEXT
:satisfies: REQ-OPAQUE-BODIES

Store handoffs as independent Markdown files under `<resolved Git common directory>/work/handoffs/`. Their frontmatter identifies `from_items` and `to_items`, with optional session and workspace references; their bodies are opaque caller-authored text. A record can cover several sources and receivers, and the same item can appear in both lists for continuation. These references deliver context and govern retention; they are not additional Work graph relations and do not affect readiness.

Handoff files own their text and receiver metadata; lookup views are derived in memory. Getting or claiming an item exposes incoming handoffs, its full body and metadata, compact relation references with titles/states, readiness, and claim/workspace context. Neighbor bodies can be fetched separately. An external runner passes the returned context to an agent. Work does not extract acceptance sections, compose an AI briefing, send messages, or launch sessions.

Persist outgoing context before releasing ownership. A failed completion keeps context; completing sources does not remove it. Automatic deletion becomes eligible when all receiving obligations are closed/resolved. Durable results needed beyond the recipients must be retained before deletion. Cancellation closes a receiver and counts as resolution. Reopening does not resurrect deleted handoffs; the next executor uses durable results and newly supplied context. Concurrent receiver changes, persistence before ownership release, and cleanup require a recoverable file protocol under [[DES-FILE-COORDINATION]]. Run cleanup must respect handoffs used outside the run.
:::

:::mara design DES-CLAIM-CONTEXT
:mid: 01M3KPWZ4FSE4HC0ZYVSQFTT4R
:title: Associate claims with sessions and persistent workspace context
:status: accepted
:kind: data
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-WORK-SELECTION

Store each acquisition as a new file `shared/claims/<claim-id>.yaml`, with claim/item identity, agent identity, required opaque session identity and namespace, store identity/generation, timestamps and optional workspace reference. Retain older claims and distinguish the one current acquisition for an item. Do not update an old claim into a new acquisition or infer the winner from timestamps. The exact release/retirement representation must be settled in the claim envelope; this does not require a generic event-log subsystem.

Owner-authorized operations carry the current claim ID and session identity. The pair identifies the acquisition and its owner; it is not a secret credential. Release, completion and reassignment make the old pair ineligible, including when the same session later reacquires the item. No separately minted ownership token or secret-token storage is introduced. Work is local cooperative coordination, not a security sandbox against a process that can edit its files.

Claim-next selects, rechecks and reserves under the shared lock. Claims are repository-wide across linked worktrees, with no automatic expiry or mandatory heartbeat. Explicit recovery follows [[REQ-CLAIM-RECOVERY]]. A claim change does not stop the external executor. Workspace records remain separate and survive release; Work records supplied context without creating worktrees.

Reusable sessions use a separate optional named-session YAML file under the run's `sessions/` directory: run scope, name, provider/namespace, and opaque external session ID. This record is the binding; no separate mandatory session registry is required. One-time workers can put an external session reference on their claim without creating a named record. Names survive individual claim release and item completion. Rebinding a name does not transfer active ownership. Availability and its observation time are optional externally reported data, not proof that a session is alive or resumable. The external runner selects fresh versus reused sessions and performs launch, dispatch, and resume operations.

Workspace reuse, run defaults, item overrides, and explicit cleanup follow [[DES-WORKSPACE-ASSOCIATIONS]]. Named sessions do not own workspaces. Remove named-session records when successful run squash or explicit finalization cleanup discards the run's execution context, not on individual item completion. Removing a name never terminates or deletes the external session. Exact assignment-hint serialization remains an engineering detail.
:::

:::mara design DES-SQUASH-DIGEST
:mid: 01M3KPWZ4WR4MM8DFR5YJ906PR
:title: Retain a run digest as an extension to its root
:status: accepted
:kind: behavior
:satisfies: REQ-RUN-FINALIZATION
:satisfies: REQ-OPAQUE-BODIES

Squash retains a caller-written summary as an extension to the existing durable root in a designated surviving checkout. Preserve the supplied digest text verbatim and preserve the root's existing content. Do not create a separate digest task, child item or completion obligation. The exact representation of the root extension is an engineering detail to settle before implementation.

The root's lifecycle remains separate from run finalization. A finished run has resolved member obligations and no active claims; an unfinished manual root can remain open after its run is finalized. Saving a digest never implicitly closes it. This operation does not promote arbitrary wisps or discard unfinished work.

Retain the digest before removing wisps or optional named-session bindings. Cleanup must refuse and identify surviving item or handoff references that still need those records; the caller resolves them or retains the files. Shared workspaces and cross-run handoffs follow their own lifecycles. Use a surviving output checkout even after a feature workspace has been removed; do not automatically commit, merge or terminate sessions.

Do not introduce template application records for finalization. Keep interruption handling bounded to retaining the root extension before cleanup, preserving evidence and allowing inspection/continuation. Exact serialization and repeat-call behavior must be specified before implementation; no general transaction engine is required.
:::

:::mara design DES-WORKSPACE-ASSOCIATIONS
:mid: 01M3KTFNTVA1JKNHD2KWH10R7V
:title: Reuse workspace records independently of sessions and claims
:status: accepted
:kind: data
:satisfies: REQ-WORKSPACE-CLEANUP
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-EXTERNAL-EXECUTION

Keep one YAML file per workspace under shared `workspaces/` as defined in [[DES-ENTITY-LIFECYCLES]], identifying an externally managed path/worktree with optional branch and commit context. A workspace can be reused by multiple activities and shared across sessions; it is not owned by a session or claim. Record a default workspace for a run and allow an item to override it. A claim records its resolved execution workspace. Associations survive individual completion and claim release.

Sharing records does not lock the underlying filesystem: item claims exclude competing ownership of an item, not concurrent writes by different items. External tooling controls execution concurrency; shared-resource locks remain deferred.

An ordinary cleanup item identifies the workspace to remove and follows the actual users of that workspace. Its external executor retains required results, performs the worktree removal, and supplies the outcome. Successful removal permits operational record/association cleanup; failure preserves retry context. Entering cleanup checks that no existing work still needs the workspace and records a closing flag that rejects new assignments to it. This guard is limited to cleanup, not general resource locking. Failed removal preserves the workspace record and retry context. The cleanup executor operates from a surviving control checkout; the target being removed is distinct from its execution workspace. Cleanup-target serialization and crash recovery after physical removal but before outcome recording remain engineering specifications.
:::

:::mara design DES-DURABLE-OPERATIONS
:mid: 01M3MM0BYYWHQQ1F76144EP45D
:title: Publish safe shared durable item operations
:status: accepted
:kind: interface
:satisfies: REQ-LOCAL-STATE
:satisfies: REQ-OPAQUE-BODIES
:satisfies: REQ-GRAPH-INTEGRITY
:satisfies: REQ-EXTERNAL-EXECUTION

The shared durable operation layer accepts a selected checkout root and returns typed outcomes independent of CLI/MCP presentation. All item references at this layer are full canonical IDs; adapters may resolve displayed abbreviations before calling it.

- `create(title, body, metadata)` returns the created item and its graph evaluation. Generate a UUIDv4 ID, publish only to `.work/items/<id>.md`, and retry an ID collision without replacing the existing entry. Once creation publishes a file, any subsequent sync, selected-path, or reload failure identifies the generated full ID and path so the caller can inspect that item before retrying.
- `inspect(id)` returns the parsed header, verbatim body bytes, source path, direct relationships including derived reverse views, and effective completion/readiness when the graph is valid. `list` returns deterministic canonical-ID-ordered inspections of valid item files while omitting malformed files. `inspect_raw(id)` returns the original source and diagnostics even when the item header, filename, or selected graph is invalid.
- `update(id, metadata)` replaces only explicitly supplied mutable header fields and returns the resulting item. Identity and body are immutable in this operation.
- `relation_add/remove(source, kind, target)` accepts `parent`, `depends_on`, `related`, or `discovered_from`; stores one full-ID assertion, returns the affected item and resulting direct relationships, and rejects duplicate/missing edges. A parent edge is authored on the child. Symmetric `related` is stored at one endpoint only; removal chooses the author from the snapshot read under the mutation lock.
- `close(id, reason?)` records `done` on a manual item and an optional opaque nonblank single-line reason. `reopen(id)` records `open` and removes any reason. Neither operation writes state on an aggregate, cascades manual state to other items, acquires a claim, or performs external execution.

A successful mutation publishes one durable file and yields the reloaded selected graph and item, so its result reflects the published state. Validate candidate source and the complete candidate graph before publication. Reject a structured graph mutation against an invalid selected graph. `repair(id, replacement_source)` is available only for an item with a source or graph diagnostic, including any member of a diagnosed cycle even when the diagnostic is displayed at another member's path. It replaces that file with a valid version-1 document bearing the same ID, refusing to reinterpret a valid parsed ID through a different filename. A diagnosed symlink or other nonregular entry is replaced without following it. A regular source whose mode lacks owner-read permission is rejected before publication with an actionable error; the caller must make it readable before retrying, because preserving that mode would make the replacement unreadable. When a malformed canonical entry shadows a uniquely identified misnamed entry for the same ID, repair first archives the misnamed entry under a hidden recovery name, then replaces the canonical entry; the result identifies both retained sources. Repair returns the reloaded source and selected graph diagnostics. The repair may leave existing graph diagnostics for later repairs, but must introduce no new ones. For a misnamed file with a valid header ID, publish the replacement at the canonical path without overwriting an existing entry, then retain the old file under a hidden recovery name; interrupted steps remain inspectable. If an error occurs after canonical publication but before the old source is archived, return the published full ID and canonical path together with the old source path and the failure cause; callers inspect both paths before retrying, without assuming the old path still exists. If publication left both the canonical and misnamed copies, `inspect_raw(id)` selects the misnamed copy and `repair(id, replacement_source)` may resume by archiving it only when the supplied replacement exactly matches the already published canonical bytes. A failure during that archival retry also returns the published full ID, canonical path, and previous source path so callers can reconcile the two entries. Reject a symlink selected checkout root, and open the items directory relative to the held, locked `.work` directory, then bind staging, snapshots, publication, rollback, and directory sync to that items handle. The `.work` pathname must remain the same real directory held for the lock, and a changed `.work` or items pathname produces a conflict without following a replacement symlink; if detected after publication, the conflict identifies the retained recovery entry in the originally opened directory. Use a same-directory staged file with restrictive permissions set explicitly despite the caller's umask, preserve the existing item's full mode on replacement, sync the stage, check that the target still matches the loaded bytes, file identity, mode, and symlink destination before publication, and use recoverable replacement; creation uses exclusive no-overwrite publication. Set the operation lock's mode explicitly and verify that its pathname still names the held lock inode before publication. A detected change before publication fails without replacing the target. Retain the replaced source under its hidden recovery path after every successful replacement and return that path in the result, since an outside editor may still write through an open handle. Recovery copies are not automatically pruned. If a direct edit races with an exchange or rollback, retain the staged recovery copy and identify it in the conflict so no concurrent content is discarded. One-file publication does not imply atomicity across several entity files. Operations that couple durable changes to shared claims or runs require [[DES-FILE-COORDINATION]] in their delivery slice.

Shared failures have stable categories: `invalid_argument` (field or relation contract), `not_found`, `already_exists` (duplicate edge or ID), `invalid_source` (malformed or invalid existing graph, with diagnostics), `invalid_candidate` (candidate graph diagnostics), `conflict` (changed-on-disk source or competing publication), and `io` (read, staging, sync, or publication failure). Errors carry a machine-readable category and human message; diagnostic failures also carry source diagnostics. Prepublication rejection leaves live files intact. A vanished exchange target is a conflict. A conflict or I/O failure after the atomic name exchange may have published the new file; the error identifies any retained recovery copy so callers can inspect it and the target before retrying. Direct file edits outside Work's lock cannot be made transactionally atomic with this one-file operation. Adapters map these categories to their transport envelopes without changing semantics.
:::

:::mara design DES-CLI-JSON
:mid: 01M3P19AKC412WF8W8TPBX82A9
:title: Expose durable item commands and JSON envelopes
:status: accepted
:kind: interface
:satisfies: REQ-CLI-JSON
:satisfies: REQ-WORKTREE-VIEWS

The durable CLI slice uses `work [--json] [--worktree PATH] COMMAND`. Options precede the command. An omitted worktree selects the caller's checkout; an explicit path selects that Git working checkout's durable files without changing the caller's branch. `discover [PATH]`, `--version`, and `--help` remain available. Help advertises only implemented commands; JSON help returns `{"help":"..."}`. This slice implements `item create --title TEXT [--body TEXT|-]`, `item list`, `item inspect ID [--raw]`, `item diagnose`, `item ready`, `item update ID OPTIONS`, `item close ID [--reason TEXT]`, `item reopen ID`, `item repair FULL_ID --source -`, and `relation add|remove KIND SOURCE TARGET`. KIND is `parent`, `depends_on`, `related`, or `discovered_from`. Create and update accept `--completion manual|children`, `--priority 0..4`, `--parent ID`, `--clear-parent`, repeatable `--label TEXT` (replacement set), `--clear-labels`, `--model TEXT`, `--clear-model`, `--thinking TEXT`, and `--clear-thinking`; update also accepts `--title`. Relationships other than parent use the relation commands. Body/source `-` reads stdin bytes. Inspect accepts full canonical IDs or unambiguous lowercase hexadecimal prefixes with optional `w-`; all mutations resolve IDs before invoking shared operations. Raw inspection and repair require a full ID to identify malformed source safely. No command in this slice claims, instantiates templates, or manages runs; those surfaces require their separate coordination contracts.

With `--json`, each invocation writes exactly one UTF-8 JSON object and a newline to stdout. Success is `{"ok":true,"result":...}`; failure is `{"ok":false,"error":{"code":"...","message":"..."}}`, with `diagnostics` on source or graph failures and optional `published_item` and `previous_source_path` when publication may have occurred. JSON paths use percent encoding of non-safe Unix bytes, matching discover output. A returned item has `id` (full canonical ID), `display_id` (`w-` plus the shortest unique prefix of at least eight hex characters in the selected view), `title`, `completion`, `state` (null for aggregates), `effective_done` and `executable` (null when the graph is invalid), `blockers`, `priority`, `parent`, `depends_on`, `related`, `discovered_from`, `labels`, `model`, `thinking`, `close_reason`, `body`, `path`, `relations` (including reverse views), `graph_diagnostics`, and `recovery_path`. Blockers are tagged objects with `kind` and full IDs where applicable. `item list` returns `{"items":[...]}` in canonical ID order; `item ready` returns `{"items":[...]}` in priority then ID order, including an empty array when nothing is ready; `item diagnose` returns `{"diagnostics":[...]}`; item mutations and inspect return `{"item":...}`. `item inspect ID --raw` and `item repair` return `{"source":{"id","path","raw_hex","diagnostics","graph_diagnostics","recovery_path","additional_recovery_paths"}}`; raw bytes use lowercase hexadecimal so malformed UTF-8 remains inspectable. A diagnostic has percent-encoded `path`, nullable one-based `line`, and `message`. The discover result has `worktree_root` and `git_common_dir`. Human output may abbreviate IDs but JSON always includes full IDs.

Stable error codes for this slice are `usage`, `invalid_argument`, `not_found`, `ambiguous_id`, `already_exists`, `invalid_source`, `invalid_candidate`, `conflict`, `unsupported_project`, and `io`. Syntax/invalid arguments exit 2; missing or ambiguous IDs exit 3; invalid source/candidate exits 4; conflict/already-exists exits 5; discovery or I/O exits 1. Success exits 0. A JSON failure uses stdout only; human failures use stderr. An invalid graph blocks ready and mutations but list, inspect, diagnose, and repair remain available. Error categories preserve the shared operation meaning; CLI parsing adds `usage` and `ambiguous_id`. Unknown commands and deferred operations return usage, never a placeholder success. Additive result fields may appear later; existing field meaning and error codes are stable within the initial version.
:::

:::mara design DES-MCP-STDIO
:mid: 01M3P4Z9P9XAPF9KC4FR0DQN9E
:title: Expose durable operations through MCP stdio
:status: accepted
:kind: interface
:satisfies: REQ-CLI-MCP-PARITY

`work mcp` serves newline-delimited JSON-RPC 2.0 over stdio, negotiating MCP protocol version `2025-06-18`. It advertises tools and no resource or prompt capabilities. Tool calls return the CLI JSON result payload as `structuredContent` and JSON text content. Domain, source, and selection failures return `isError: true` with `{"error":{"code","message",...}}` in structured content; unknown protocol methods and tool names use JSON-RPC errors. stdout contains only protocol messages.

The first slice advertises `discover`, `item_create`, `item_list`, `item_inspect`, `item_inspect_raw`, `item_diagnose`, `item_ready`, `item_update`, `item_close`, `item_reopen`, `item_repair`, `relation_add`, and `relation_remove`. Every tool accepts optional `worktree` (a Git working checkout path); omission selects the server process checkout. Item references use `id`, relation calls use `source`, `kind`, and `target`, and the shared core receives resolved full IDs. Raw repair uses `raw_hex` to preserve arbitrary source bytes. Create accepts required `title` and optional `body`; create and update accept `completion`, `priority`, `parent`, `labels`, `model`, and `thinking`. Update accepts required `id` and optional `title`. Explicit `clear_parent`, `clear_model`, and `clear_thinking` booleans clear those optional fields; an empty `labels` array clears labels. Close accepts required `id` and optional `reason`; reopen and both inspections require `id`. Relation kind is one of `parent`, `depends_on`, `related`, or `discovered_from`.

Each `inputSchema` is an object with `additionalProperties: false`, required fields as stated, and JSON types: strings for text and IDs, integer 0–4 for priority, string array for labels, booleans for clear flags. Optional fields are absent when unset; explicit null or wrong-type values fail as `invalid_argument`. Conflicting set and clear inputs fail likewise. No tool in this slice claims work, starts a run, launches an agent, or manages a worktree. Tool names and schemas may grow in later slices without changing the semantic meaning of existing fields.
:::
