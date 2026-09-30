# Verification definitions

These are repeatable acceptance definitions, not execution results. The implemented durable-item slice has automated checks in `tests/`, including copied-backlog adoption in `tests/bootstrap_adoption.rs`; consult actual test runs for passing evidence and limitations. Checks for later slices remain planned.

:::mara verification VER-DURABLE-ROUNDTRIP
:mid: 01M3KARN7CQA58Z2TFCT9BHMYM
:title: Inspect durable state across sessions
:status: accepted
:method: demonstration
:verifies: REQ-LOCAL-STATE
:verifies: REQ-DURABLE-FILES
:validates: SCN-PLAN-WORK

Create a work item with a multiline description, acceptance criteria, a dependency, and a recorded outcome. Exit every Work process and reopen the project in a new session. Inspect the item through Work and directly in the durable YAML/Markdown. Expect the same semantic content and recorded state, with no chat-history dependency and every retained field readable from the item files.
:::

:::mara verification VER-DEPENDENCY-SELECTION
:mid: 01M3KARN7N16ZYKS4Q30X1Y5YJ
:title: Demonstrate dependency-driven selection
:status: accepted
:method: demonstration
:verifies: REQ-WORK-GRAPH
:verifies: REQ-ACTIONABLE-WORK
:validates: SCN-NEXT-WORK

Create A and B with B blocked by A, plus independent C. Before A satisfies the prerequisite, B is excluded from actionable work and the blocker is inspectable; C remains eligible. After recording the successful prerequisite outcome for A, B becomes eligible if its own state allows work. Repeat with multiple prerequisites and require all blocking prerequisites to be satisfied.
:::

:::mara verification VER-TEMPLATE-GRAPH
:mid: 01M3KARN7XH85GXF835QFQHTDJ
:title: Demonstrate ordinary graph items from a template
:status: accepted
:method: demonstration
:verifies: REQ-GRAPH-TEMPLATES
:validates: SCN-REPEATABLE-REVIEW

Instantiate a small template with implementation, review, and a delivery condition. Inspect the created identities and edges, then use ordinary item inspection and dependency selection on each node. Expect the declared ordering and the same graph semantics as manually authored equivalent items. Completing implementation alone must not be represented as completion of remaining steps.

Supply template parameters and preview the resulting bodies, metadata, and graph. Verify preview leaves item files and claims unchanged, then instantiate and compare the semantic output with the preview. Verify later metadata edits preserve the rendered body verbatim. Expand a material-only template during planning without a run, then a mixed template with a target run: material files go to `.work/items/` and wisps to run `items/`, with ordinary graph relationships between them. No template-level planning/execution category or application record is created. The per-item persistence selector is a planned format extension, not part of the already delivered preview checks.
:::

:::mara verification VER-QUIET-RUNTIME
:mid: 01M3KARN85TADDXC48385KJTEH
:title: Inspect the durable and operational boundary
:status: accepted
:method: demonstration
:verifies: REQ-EPHEMERAL-WORK
:verifies: REQ-SIDE-STATE
:validates: SCN-QUIET-EXECUTION

Capture durable file contents and instantiate temporary execution as separate run and wisp files. Create claims, named sessions, reusable workspace records and handoffs in their documented locations, then restart every Work process. Inspect each entity directly with ordinary file tools and through CLI/MCP; expect matching semantic content and unchanged ownership. No SQLite file or single all-entity snapshot is required, and operational updates do not dirty versioned content. Retain a feature-level digest extension on the root through ordinary Git commit/squash history before explicit temporary cleanup. Confirm branch merge alone does not copy live operational files into main.
:::

:::mara verification VER-CLAIM-RACE
:mid: 01M3KARN8D5NW801AXBVV5GEWZ
:title: Race claims and reject stale owners
:status: accepted
:method: test
:verifies: REQ-CLAIM-EXCLUSION
:verifies: REQ-CLAIM-RECOVERY
:validates: SCN-CONCURRENT-AGENTS

Use independent processes in linked worktrees to claim the same item simultaneously. Assert exactly one owner and an explicit conflict for the loser, even when callers supply different run/worktree contexts. Readers must remain able to inspect progress. Claim distinct items and expect independent ownership. Explicitly release and reassign a claim, then submit completion and release requests with the former claim ID plus session identity and expect rejection. Recheck readiness between listing and claiming. Advance time without heartbeats and verify claims do not expire. Exercise explicit controller recovery of abandoned ownership. Exercise concurrent independent CLI and MCP processes with notifications disabled or missed. Kill a process while it holds the shared OS lock; a later process can acquire the lock but cannot claim an already-owned item. With executors stopped, corrupt a claim file in a disposable fixture and verify an explicit storage warning/refusal rather than treating it as unclaimed. Explicit repair or ownership recreation rejects former claim/session authorizations and never invents prior owners. Reacquire the same item from the same session and require a new claim ID; the old pair remains rejected. Inspect retained prior claims and verify that new ownership never overwrites their acquisition identity. Reversed or equal caller timestamps cannot select a different winner. No separate secret token or token store is required.
:::

:::mara verification VER-WORKTREE-ISOLATION
:mid: 01M3KARN8NJYGF8F14YQAFKD71
:title: Query divergent durable views independently
:status: accepted
:method: test
:verifies: REQ-WORKTREE-VIEWS
:verifies: REQ-INDEX-REBUILD
:validates: SCN-REBUILD-INDEX

Create linked worktrees whose same-ID durable item differs in content and completion, including uncommitted changes. Alternate queries and source reloads from each tree. Without active-run state, expect the selected checkout's recorded state. Add active-run state and expect normal inspection/readiness from both checkouts to use it, while retaining each checkout's own body and relationships and exposing the recorded/effective state source. Querying the overlay must not rewrite durable files. Finalizing a finished run must not close an open root. Shared claims exclude competing execution across both trees. Restart and reconstruct the in-memory graph with claims, runs, sessions, workspaces and handoffs present; compare authoritative file bytes and require unchanged ownership/context.

From a control checkout select each worktree by path and inspect its content without switching branches. Exercise branch switching and externally removing an eligible feature worktree; shared run/claim files remain inspectable and cannot be recreated from stale branch-local claim copies. The operational workspace record follows explicit cleanup reporting. In a separate stopped-execution fixture corrupt or remove recognizable initialized store metadata: require a warning for available file inspection/readiness, blocked claims, preserved surviving entity files and an explicit recovery path.
:::

:::mara verification VER-RUN-RECOVERY
:mid: 01M3KARN8Y3FYNPDNEKTVK2J1A
:title: Inspect partial expansion and preserve results before cleanup
:status: accepted
:method: test
:verifies: REQ-RUN-ATOMICITY
:verifies: REQ-RUN-FINALIZATION

Validate invalid template inputs without creating files. For a valid five-item expansion, inject failure after three creations. Inspect those files and the returned created-ID mapping and failure; no application record or transaction receipt exists. Remove the partial result as the caller and retry to create the intended graph. Repeat for a mixed material/wisp template, including restart after a killed creator; surviving files remain discoverable without a promise of automatic rollback or duplicate suppression. Ordinary malformed-file or unresolved-reference diagnostics still apply.

For finalization, inject interruption before root-digest retention and before wisp cleanup. No temporary content may be discarded before the digest is retained. Inspect and continue using the bounded finalization interface. Keep an outside reference to verify cleanup identifies the blocker. The root's manual completion stays unchanged. Do not assume atomic rename across the surviving checkout and shared storage. A process-kill test is not proof of power-loss durability.
:::

:::mara verification VER-GRAPH-ERRORS
:mid: 01M3KARN9774XWFJWWJ3G40T2W
:title: Exercise invalid graph input
:status: accepted
:method: test
:verifies: REQ-GRAPH-INTEGRITY

Exercise duplicate identities, missing prerequisite IDs, self-blocking, a multi-node cycle, and malformed durable input. Expect actionable diagnostics and no false-ready classification for affected work. Repeat through structured mutation and direct file editing; rejected structured edits must preserve the previous source, while direct edits remain visible as errors rather than being silently overwritten.
 Include a disconnected otherwise-ready component: an invalid selected graph must refuse readiness and claims even for that component, while inspection and repair remain available. Verify another valid project continues normally.
:::

:::mara verification VER-CLI-CONTRACT
:mid: 01M3KARN9EV02H2NTB1JW5K7QQ
:title: Exercise structured CLI behavior
:status: accepted
:method: test
:verifies: REQ-CLI-JSON

Exercise [[DES-CLI-JSON]] through the executable and parse stdout as JSON: create, inspect/list, update, relate, ready, close/reopen, diagnose, and repair in disposable Git fixtures. Assert full canonical IDs, human display prefixes, selected-worktree isolation, an empty ready array, invalid arguments, missing and ambiguous IDs, malformed source, invalid graphs, and conflicting writes with documented codes and exit statuses. Confirm one JSON object on each success or failure and no deferred command advertising. Later CLI slices extend this check to claims, templates, temporary runs, and process restart.
:::

:::mara verification VER-GATE-REVISION
:mid: 01M3KARN9PSYMR71K9MVYH9Y4X
:title: Reject outdated or unavailable gate observations
:status: retired
:method: test
:verifies: REQ-GATE-CONTEXT

Retired with REQ-GATE-CONTEXT. This proposed check assumed a Work-owned external observation evaluator. Review/CI interpretation belongs to the external executor; Work's checks concern item completion and graph-based blockers rather than provider outcome classification.
:::

:::mara verification VER-SURFACE-PARITY
:mid: 01M3KATNCCYRG9WDYEWETDH02K
:title: Compare CLI and MCP operation semantics
:status: accepted
:method: test
:verifies: REQ-CLI-MCP-PARITY

Use equivalent isolated fixtures to perform core item, relationship, readiness, claim, template, and run operations through CLI and MCP. Normalize transport envelopes and compare resulting files, coordination state, semantic results, and failure meanings. Cover invalid inputs and claim conflicts as well as success. Expect both surfaces to invoke the same product behavior.
:::

:::mara verification VER-EXECUTION-BOUNDARY
:mid: 01M3KATNCP9X11F0JEZZHC1BP2
:title: Inspect the tracking and execution boundary
:status: accepted
:method: test
:verifies: REQ-EXTERNAL-EXECUTION

Exercise item selection, claiming, run creation, and supplied workspace or PR/CI outcome recording with external executables and network calls instrumented. Expect these operations not to launch agents, create worktrees, open or merge pull requests, or trigger CI. Inspect the operation catalog to ensure no initial-release command or MCP tool performs those external activities.
:::

:::mara verification VER-EPHEMERAL-FILES
:mid: 01M3KATNCZAAZ9E3W5T7EAYES4
:title: Recover temporary work content from files
:status: accepted
:method: test
:verifies: REQ-EPHEMERAL-FILES

Create a run with multiple wisps and active execution state, including items created by repeated template expansion. Inspect the separate manifest and entity files directly; no application/provenance entity is required. Restart Work and rebuild its disposable view without any database or persistent index; expect all content, edges, active execution state and ownership records to remain. In a stopped-execution fixture corrupt one entity file and verify diagnostics identify its path and preserve the other entities. Temporary cleanup remains an explicit lifecycle operation, never a side effect of rebuilding a graph.
:::

:::mara verification VER-NESTED-DELIVERY
:mid: 01M3KFAJ5D66M6KQSWBVR9HCP1
:title: Exercise nested completion and inherited prerequisites
:status: accepted
:method: test
:verifies: REQ-PARENT-COMPLETION
:verifies: REQ-HIERARCHICAL-READINESS
:validates: SCN-NESTED-DELIVERY

Construct [[SCN-NESTED-DELIVERY]] with A1 and A2 under manual A, and A plus deployment B under aggregate C; B depends on A. Assert A1 and A2 are eligible together, A remains non-executable until both finish, and B waits for explicit completion of A. After B completes, C derives done and is never claimable. Add an explicit prerequisite to C and verify it gates descendants until satisfied without propagating the internal child wait. Verify empty aggregates stay open, nested aggregates compose, and adding or reopening an unfinished child makes an aggregate incomplete. Verify a manual leaf needs no children and can execute normally.
:::

:::mara verification VER-INFORMATIONAL-RELATIONS
:mid: 01M3KFAJ5SS5PRR2FN7RYK5HCF
:title: Verify informational edges do not schedule work
:status: accepted
:method: test
:verifies: REQ-INFORMATIONAL-RELATIONS

Capture readiness and completion for independent items, then add related and discovered_from links. Expect unchanged lifecycle results, symmetric related navigation, and correctly directed discovery provenance. Add a lifecycle dependency alongside discovery provenance for the same pair: only the dependency changes readiness; removing it restores eligibility while preserving discovery context.
:::

:::mara verification VER-RELATION-STRUCTURE
:mid: 01M3KFAJ64V5HQZFGBQZBZPQN4
:title: Reject lifecycle deadlocks and derive reverse views
:status: accepted
:method: test
:verifies: REQ-RELATION-STRUCTURE

Reject a second parent, a parent cycle, and a pure dependency cycle. Reject a child depending on its own parent and a parent depending on its own child, since complete outcomes and inherited prerequisites would deadlock. Reject a longer mixed hierarchy/dependency deadlock. Accept ordinary parent-child aggregation without treating its internal child wait as a downward blocker. Inspect children and blocks from single authored edges, and symmetric related navigation from one edge.
:::

:::mara verification VER-HANDOFF-RETENTION
:mid: 01M3KPXXA1FN9H5ZFKEPEJTJH9
:title: Verify context survives until receiving work finishes
:status: accepted
:method: test
:verifies: REQ-HANDOFF-CONTEXT

Create a handoff from five completed fix items to one consolidation item, and another with two receiving items. Getting and claiming a receiver must return the original opaque body and source/workspace references. Completing sources, releasing ownership, reassigning ownership, or failing completion must retain the handoff. Complete one of two receivers and expect retention; complete both and expect automatic cleanup eligibility. Exercise a same-item continuation. Verify handoff references never affect readiness, run cleanup preserves externally needed context, and no agent is launched. Cancel a remaining receiver by closing it with a reason and expect cleanup eligibility. Reopen a receiver after cleanup and verify Work returns retained durable context without fabricating the deleted handoff.
:::

:::mara verification VER-OPAQUE-CONTENT
:mid: 01M3KPXXAFCWZ3YSNZVEGY5AGX
:title: Verify verbatim bodies and optional executor hints
:status: accepted
:method: test
:verifies: REQ-OPAQUE-BODIES
:verifies: REQ-EXECUTOR-HINTS

Use an item body with unconventional headings, checkboxes, code blocks, and no acceptance section. Update metadata and assert body bytes are unchanged. Retrieve item context and expect the full original body rather than extracted sections. Store supplied handoff and digest text and compare it verbatim. Exercise items with neither, one, and both optional model/thinking fields; expect acceptance without a provider catalog lookup or agent launch.
:::

:::mara verification VER-SELECTION-CONTEXT
:mid: 01M3KPXXAXJK7Q4AEQZ2ED7NAM
:title: Verify scoped atomic selection and graph explanations
:status: accepted
:method: test
:verifies: REQ-WORK-SELECTION
:verifies: REQ-GRAPH-INSPECTION

Once concrete priority and filter schemas are selected, construct eligible, dependent, inherited-blocked, manual-parent, aggregate, and already-claimed items. Check priority ordering and stable ties within the requested scope. Race claim-next from independent processes and assert no duplicate ownership and explicit empty results when exhausted. Compare structured readiness explanations and graph inspection against the same evaluated state used for selection. Verify an earlier explanation does not reserve an item and eligibility is rechecked at claim time.
:::

:::mara verification VER-SQUASH-DIGEST
:mid: 01M3KPXXBAZ20409CEYQGVGMRD
:title: Verify retained digest before temporary cleanup
:status: accepted
:method: test
:verifies: REQ-RUN-FINALIZATION

Supply a finished run and caller-written summary while its manual root remains open. Finalization retains the summary verbatim as an extension to that existing root, preserving prior root content and completion state and creating no new digest item or child edge. Verify retention precedes wisp cleanup, including interruption and inspection/continuation. Reject finalization of unfinished member work or work with active claims. Keep surviving item references and handoffs with unresolved receivers to verify cleanup identifies blockers and preserves needed context. Root completion remains a separate explicit operation. Exact repeat-call behavior follows the bounded finalization contract.
:::

:::mara verification VER-WORKSPACE-CLEANUP
:mid: 01M3KTFNV9ACZ6MTYX9WVGGPD7
:title: Verify reuse and explicit cleanup across workspace users
:status: accepted
:method: test
:verifies: REQ-WORKSPACE-CLEANUP
:verifies: DES-WORKSPACE-ASSOCIATIONS

Associate sequential activities and different sessions with a shared workspace; verify completion and release preserve the workspace association. Exercise a run default, an item override, and the resolved workspace on a claim. Add another unfinished user outside the originating run and verify cleanup cannot proceed while that user needs the workspace. Verify required downstream context is retained before removal. Have external tooling report removal failure and expect inspectable context and a retryable cleanup item; report successful removal and expect operational workspace record cleanup. Instrument Work's operations and verify they do not perform actual worktree deletion. Race and interruption cases must follow the cleanup protocol once its detailed design is selected.
 Enter the closing state only after checking existing users; attempt a new assignment while closing and expect rejection. Run cleanup from a surviving control checkout and verify shared run files remain accessible after feature-worktree removal.
:::

:::mara verification VER-SESSION-REUSE
:mid: 01M3KTFNVQNWAXJ9B11M3VWP3D
:title: Verify optional named-session reuse without transferring ownership
:status: accepted
:method: test
:verifies: DES-CLAIM-CONTEXT

Create a run-scoped implementer name pointing directly to an opaque provider/session reference. Complete its first item and verify the name remains available for a later item, which acquires a new claim. Use the same name in another run and verify scope isolation. Claim one-time fix items with external session references and no named-session registration. Rebind or remove a name and verify this neither transfers an existing claim nor lets the replacement session authorize operations using the former claim. Treat availability as a timestamped external observation, not proof that a session remains available. Verify no session is launched or resumed by Work.
 Successfully squash or explicitly clean finalized execution context and verify its named-session records are removed without terminating external sessions.
:::

:::mara verification VER-SINGLE-RUN
:mid: 01M3KVG3J8NQWE6VA1WZV5VBA9
:title: Reject competing runs for the same root
:status: accepted
:method: test
:verifies: REQ-SINGLE-RUN

Attempt to initialize execution of the same root item from two linked worktrees concurrently. Expect one run and an explicit existing-run result or conflict, not two executions. Verify read-only inspection from the other caller succeeds. Add supporting template work and independently claim different child items within that run.
:::

:::mara verification VER-GRAPH-PROGRESS
:mid: 01M3KVG3JQ7CHW8099KJBQGVVS
:title: Verify review findings block downstream work through edges
:status: accepted
:method: test
:verifies: REQ-GRAPH-PROGRESS

Build a review followed by delivery. In the findings case, create fixes, consolidation, and another review and connect downstream delivery to the new review before completing the old review. Expect delivery to remain blocked until the new prerequisites finish. Interrupt before old-review completion and expect it to remain a blocker. In the clean case, complete the review without additional work and expect delivery to become eligible. Supply arbitrary opaque review text and verify readiness depends on item states and edges rather than interpreting that text.
:::

:::mara verification VER-GIT-CHECKOUT
:mid: 01M3KVG3K6JCE1JDCKRX1NSFMS
:title: Validate Git checkout scope and common-directory discovery
:status: accepted
:method: test
:verifies: REQ-GIT-CHECKOUT

Run project discovery in a normal Git checkout and a linked worktree and verify their expected working-tree paths and shared Git common directory. Try a non-Git folder and a bare repository and expect an explicit unsupported-project error without initializing Work runtime storage.
:::

:::mara verification VER-SIMPLE-LIFECYCLE
:mid: 01M3KWDF89D6HY6338FS1VFW3F
:title: Verify closure and reopening without cascading execution
:status: accepted
:method: test
:verifies: DES-LIFECYCLE

Close a prerequisite with a cancellation reason and verify its obligation is resolved for ordinary dependents and aggregate parents. Reopen it and verify open dependents become blocked and aggregates recompute; completed manual parents and dependents must not reopen automatically. Confirm that Work does not interpret closure-reason text. Reopen a retained item after handoff/workspace cleanup and verify no deleted operational resources are recreated.
:::

:::mara verification VER-ITEM-IDENTITY
:mid: 01M3KWDF8QDP00X8QNWAFSHCD8
:title: Verify immutable identities and unambiguous abbreviations
:status: accepted
:method: test
:verifies: DES-FILE-LAYOUT

Create durable and temporary items and verify UUIDv4 canonical identities use 32 lowercase hexadecimal characters, with item files named by the canonical identity and stored relations using full IDs. Rename titles and reparent items without changing identities. Construct ambiguous short prefixes and require longer input rather than arbitrary resolution. Verify no generated item overwrites an existing canonical ID and no sequential alias or hierarchy is encoded in the identity.
:::

:::mara verification VER-BOOTSTRAP-COMPATIBILITY
:mid: 01M3KY9FJ6J91RHHNG82YJEQZN
:title: Adopt real bootstrap items through both interfaces without rewriting bodies
:status: accepted
:method: test
:verifies: REQ-BOOTSTRAP-COMPATIBILITY
:verifies: DES-ITEM-FORMAT

Copy the real .work/items backlog into a disposable initialized Git checkout. Load it without importing to a database or changing IDs/body bytes. Confirm aggregate items omit recorded state, manual items have valid state, and every full-ID relationship resolves. Exercise creation, inspection, readiness, relationships, close, and reopen through CLI and MCP on equivalent fixtures, preserving original files and comparing semantics. Verify the expected initial ready item and the next ready item after prerequisite completion. Cover LF/CRLF bodies, ambiguous YAML strings, duplicate keys, unknown fields/versions, invalid IDs, filename mismatches, aggregate stored state, unresolved edges, reciprocal related assertions, and mixed lifecycle deadlocks. Rejected writes must preserve files. The delivered durable-item slice has automated coverage in `tests/bootstrap_adoption.rs` and other integration tests in `tests/`. This definition records the repeatable check, not a passing result; consult actual test runs for execution evidence.
:::

:::mara verification VER-FILE-COORDINATION
:mid: 01M3RPMWJSJ2SVH8XKQKTV5SGX
:title: Verify separate entity storage and shared file coordination
:status: accepted
:method: test
:verifies: DES-SHARED-FILES
:verifies: DES-ENTITY-LIFECYCLES
:verifies: DES-FILE-COORDINATION

Use disposable Git repositories and linked worktrees to verify the entity layout, one authoritative owner per field/reference, and independent retention rules. Run concurrent CLI/MCP mutations through the shared lock, including contention and process termination. Confirm lock-file presence is not interpreted as ownership and normal cleanup never unlinks/replaces the live lock inode. Test symlink/path substitution and source conflicts without following unsafe replacements or silently overwriting changed content.

Inject failure before and after single-file publication and directory sync; report whether publication may have occurred and retain recovery context. Inspect partial template expansion as a caller-visible result under [[VER-RUN-RECOVERY]], with ordinary graph diagnostics and no template application receipt. The foundation's own journaled storage operations retain their existing availability/recovery contract. Verify a durable-only graph rebuild never clears claims or rewrites operational files. For the foundation, uninitialized, missing/corrupt/unreadable or pending storage must report coordination unavailable instead of a healthy empty store. This is a storage-status check, not a claim-acquisition test. Actual claim refusal and malformed claim-envelope handling belong to [[VER-CLAIM-RACE]] when the claim implementation is delivered. Permission/I/O, malformed entity and lock-contention failures must be distinguishable through both adapters.

For the foundation, test that detected storage loss is reported and recreation requires explicit action with loss reporting; preserved recovery files must not appear as live entities. Backup creation and restoration are deferred. When later entity-aware restore is implemented, test matching active/retained local identity evidence and refusal without trusted local evidence, with stopped executors and former-claim/session rejection. Raw external deletion and bypassing advisory locks remain outside the stated exclusion guarantee; this test must not imply otherwise. Use separate evidence for OS-crash/power-loss durability if such guarantees are claimed.

For [[DES-STORE-FOUNDATION]] and [[DES-STORAGE-API]], exercise all four operations through real CLI and MCP subprocesses. Check fresh no-write reads, linked checkout identity, divergent item views across repeated queries/branch switches, independent clone identity, explicit initialization and healthy idempotency. Verify format/identity errors, duplicate keys, unsafe special files, hardlinks, permissions, lock contention and human warnings. Kill a lock holder and confirm descriptor release; recreation/recovery preserve an intact lock inode.

Inject failures at staged write/sync, prepared intent, witness-first publication, metadata-last publication and receipt phases. Initialization recovery reuses the same identity/generation. Recreation retains opaque entity bytes and prior operation records before creating empty live directories; interrupt after each archival step and verify idempotent same-ID resume. Reject stale expected evidence, unsupported metadata versions, conflicting live/archive locations and replay of a receipt after a later generation. Incomplete structure always exposes unavailable status. Requesting deferred backup/restore must not advertise a tool or write state. Test destructive scenarios only in disposable fixtures; opaque fixture preservation and generation changes do not establish actual claim enforcement.
:::
