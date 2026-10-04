# Beta.2: acceptance scenarios and verification

These scenarios and repeatable checks define required future behavior. They are not recorded passing runs. The [accepted wire specification](beta2-wire.mara.md) and [closed catalog](schemas/beta2-wire-v2.schema.json) freeze the fields, CLI/MCP inputs/results and failure/continuation contracts used by these checks. `node scripts/check-beta2-wire.mjs` checks specification consistency and `node --test tests/beta2_wire_spec.mjs` checks that checker; neither is runtime or package acceptance. Execute state-changing checks on disposable repositories; capture actual source revisions and CLI/MCP outcomes before reporting implementation evidence.

## Scenarios

:::mara scenario SCN-SCOPED-EXECUTION
:mid: 01M439VER70HAYSQYTHC615P2T
:title: Execute inherited and nested work scopes
:status: accepted
:contributes_to: GOAL-GRAPH-PROCESS
:involves: ACT-OPERATOR
:involves: ACT-AGENT

The primary checkout hosts a virtual root and a standing default run. Two parentless material items use that context without authored parent changes or linked worktrees. External agents claim different eligible items concurrently. A feature establishes a dedicated checkout but continues using the inherited run; an experiment beneath it establishes its own run. Unqualified session names resolve only in each item's effective run. Outcome: work ancestry, hosting, and execution namespaces compose without individual workspace bindings or explicit run-member lists.
:::

:::mara scenario SCN-SCOPE-RELOCATION
:mid: 01M439VESFABPP8GAVY75PQJZX
:title: Continue the same run after merging and relocating a scope
:status: accepted
:contributes_to: GOAL-LOCAL-WORK
:involves: ACT-OPERATOR
:involves: ACT-AGENT

An external tool commits scope B's versionable run content and merges branch B into checkout A. Work still resolves B through its declared authoritative host until an explicit relocation validates the destination and changes hosting. With caller attestation that affected executors are quiescent, B's run, named-session identities, wisps and current claim IDs continue unchanged in A. A can host both A and B. Missing or duplicate hosting declarations produce candidates/diagnostics, never automatic newest-copy selection. Outcome: merge can carry execution state without duplicating ownership or restarting the run.
:::

:::mara scenario SCN-SESSION-DRAIN
:mid: 01M439VETRAHWGZ79KH5EZ91XC
:title: Drain an immutable named session without transferring ownership
:status: accepted
:contributes_to: GOAL-LOCAL-WORK
:involves: ACT-OPERATOR
:involves: ACT-AGENT

A run registers reviewer with one actor/namespace/external-ID tuple. Acquisition and owner mutations use the readable name rather than repeating the tuple. Retirement immediately rejects new acquisitions while existing claims can finish or release; the session becomes retired only after all its current claims end, including claims in other qualified contexts. Restart, elapsed time and availability observations do not end ownership. The same run cannot reuse reviewer for a replacement identity. Outcome: stable names remain trustworthy ownership references.
:::

:::mara scenario SCN-SENDER-HANDOFF
:mid: 01M439VEVYSG8F71YN9D4JE5G5
:title: Discover sender-local addressed context across runs
:status: accepted
:contributes_to: GOAL-GRAPH-PROCESS
:involves: ACT-OPERATOR
:involves: ACT-AGENT
:supersedes: SCN-HANDOFF

A worker saves opaque context in its sending run for an item, immutable named session, or run, including a parent orchestrator or explicitly qualified sibling-run recipient. Item inspection/acquisition and filtered inbox/list operations expose incoming context through the scope resolver. The parent may read a child-local final report and later dispose the child even without reading or archiving all messages. No push delivery, acknowledgement, dependency edge or receiver-based disposal gate is introduced. Outcome: destinations affect discovery, not location or lifetime.
:::

:::mara scenario SCN-SCOPE-DISPOSAL
:mid: 01M439VEX5T94RV38MS6PX1FAY
:title: Promote useful work and dispose a child subtree bottom-up
:status: accepted
:contributes_to: GOAL-QUIET-HISTORY
:involves: ACT-OPERATOR
:involves: ACT-AGENT
:supersedes: SCN-WORKSPACE-CLEANUP

An orchestrator may promote an unclaimed wisp, retaining its item ID, state, body and relationships, then commit and merge the material item normally. A completed run may end with or without a caller-authored digest; a parent ends successfully only after required work and child runs resolve. Alternatively the parent explicitly abandons a child execution subtree: validate the bounded target set, refuse current claims and unintended material deletion, then dispose descendants before ancestors. No handoff-read or archival check is required. Physical checkout removal remains external and cannot remove a checkout still needed by another hosted scope. Outcome: scope-owned lifetimes reduce bookkeeping without treating abandonment as completion.
:::

:::mara verification VER-SCOPE-ROUTING
:mid: 01M439Z8N60CN6QADD5HMZHBJZ
:title: Verify implicit scopes and authoritative relocation
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-SCOPE-TREE
:verifies: REQ-SCOPE-HOSTING
:verifies: REQ-VERSIONED-EXECUTION
:validates: SCN-SCOPED-EXECUTION
:validates: SCN-SCOPE-RELOCATION
:supersedes: VER-QUIET-RUNTIME
:supersedes: VER-WORKTREE-ISOLATION
:supersedes: VER-EPHEMERAL-FILES
:supersedes: VER-FILE-COORDINATION

Use real primary/linked Git checkouts, with the primary on main, another branch and detached HEAD. Confirm parentless item headers are unchanged, virtual root is non-executable, inherited scope selection works at several depths, and only explicit boundaries create anchors. Commit and externally merge a live child's scoped execution files into a parent: copies alone do not move authority. Relocate under quiescence attestation and continue the same run/session/claim IDs from the destination with unchanged opaque bodies. Diagnose missing hosts, duplicate host declarations, unavailable source files and divergent destination state; show candidate checkouts without choosing the newest. Repeated reads/restart do not create state or change routing. Verify physical checkout copies can be inspected explicitly without becoming live owners.

Attempt mutations through explicit physical-copy inspection and verify rejection/no writes; normal mutations still use the declared authoritative host. Dispose execution in a dedicated checkout containing surviving material work and confirm the hosting-only boundary and material source remain unchanged. External checkout removal remains blocked until material is retained and hosting explicitly relocated.
:::

:::mara verification VER-SCOPED-RUNS
:mid: 01M439Z8PH1HVVJSAXSSSCZ195
:title: Verify default runs, nested context and derived membership
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-SCOPED-RUNS
:verifies: REQ-SCOPE-TREE
:verifies: REQ-GRAPH-TEMPLATES
:verifies: REQ-RUN-ATOMICITY
:validates: SCN-SCOPED-EXECUTION
:verifies: REQ-EPHEMERAL-WORK
:supersedes: VER-SINGLE-RUN

Initialize a disposable repository and confirm a standing root run exists without creating a synthetic task. Race initialization/start callers and verify one current run per established boundary. Root rotation creates a fresh default without requiring all repository material obligations done. A child checkout alone inherits the same run and uses that run's one storage home; a child run establishes its own namespace and descendants. Membership follows actual ancestry with no attach/detach registry; cross-run prerequisites still block work correctly. Successful non-root finalization refuses unfinished required work, current ownership and current descendant runs while leaving the root item unchanged. Verify material-only template planning requires no execution initialization, previews write nothing, and mixed expansion reports inspectable partial publication instead of application records or implicit rollback.

End a child round and verify no automatic ancestor fallback. Start a fresh round or explicitly resume inheritance, preserving dedicated hosting and rejecting affected current ownership; retained old wisps remain in their original round and do not join the new one. Root rotation with unfinished root wisps refuses unless abandonment is explicit, retains their open recorded state/history, and still refuses claims or live descendant runs. Validate child-run template wisps through declared parentage: reject disconnected/mismatched targets before writes, never add edges, and accept parentless virtual-root wisps. Ended wisps are inspectable/promotable/disposable, not editable, reopenable or claimable.

Attempt to reparent a current wisp beneath a sibling's different run and confirm rejection/no writes; same-run reparenting remains available with ordinary safety checks. Explicitly promote the unclaimed item and confirm material reparenting is available without introducing a wisp-transfer operation.
:::

:::mara verification VER-IMMUTABLE-SESSIONS
:mid: 01M439Z8QTPPNVJKYCQN19837V
:title: Verify named ownership, drain and cross-checkout exclusion
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-IMMUTABLE-SESSIONS
:verifies: REQ-SCOPED-OWNERSHIP
:verifies: REQ-CLAIM-EXCLUSION
:verifies: REQ-WORK-SELECTION
:validates: SCN-SESSION-DRAIN
:validates: SCN-SCOPE-RELOCATION
:supersedes: VER-CLAIM-RACE
:supersedes: VER-SESSION-REUSE

Register named actor/namespace/ID tuples and use the readable names for acquire, claim-next, owner mutations, release/reassign and handoff context. Reject mixed explicit/shortcut inputs, identity rebinding, retired-name reuse, and implicit ancestor fallback. Race claim-next across CLI/MCP processes and sibling hosts: only one current claim per item. Named owner mutation resolves current acquisition under the same lock; an explicitly stale acquisition ID fails. Retiring immediately refuses new acquisition while permitting completion/release; all current claims of that session must end before retired, including qualified cross-run uses. Availability observations, restart and clocks cannot release claims. Preserve current claims during validated quiescent hosting relocation, but refuse reparent/new-child-run changes that alter their effective context. Recover stopped ownership explicitly and retain acquisition/end history.

Use identical readable names with different tuples in sibling runs. Unqualified named claim-next selects within the single explicitly/structurally resolved namespace, excluding other effective runs without hiding their prerequisite effects; a qualified identity may select more broadly. Ending a run automatically retires idle active/retiring names, but refuses while one owns a claim in another run. Rejected ending must not release or rewrite that claim.
:::

:::mara verification VER-SENDER-HANDOFFS
:mid: 01M439Z8S4VDP5JPSZHDEHVPST
:title: Verify pull discovery without receiver retention obligations
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-SENDER-HANDOFFS
:verifies: REQ-OPAQUE-BODIES
:validates: SCN-SENDER-HANDOFF
:supersedes: VER-HANDOFF-RETENTION

Save verbatim bodies for local item/session/run destinations, parent reports and qualified sibling-run targets. Confirm all message files remain in the sending run, including messages to parent. Real item inspection/acquisition and filtered inbox/list expose addressed context from authoritative host files; an already-working agent learns nothing without another Work read. Completion/release and receiver state do not rewrite body or create graph edges. Explicitly dispose a sender's eligible run/subtree with unread recipients and verify no receipt, upward copying, archive, receiver-completion or acknowledgement gate blocks it. Missing sender content is exposed as unavailable context where detectable, not acknowledged delivery. No notification service or watcher is advertised.

Carry a material recipient into a fresh round and confirm item-addressed incoming context still follows its stable ID with original sender provenance. Promotion/relocation preserve discovery. Exact run/session-addressed messages do not appear as messages for replacement identities. Publish a final report before ending its sender and confirm terminal execution rejects new outgoing messages without writes.
:::

:::mara verification VER-PROMOTION-DISPOSAL
:mid: 01M439Z8TDT7NH69Q07D4PQN6H
:title: Verify identity-preserving promotion and bottom-up disposal
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-WISP-PROMOTION
:verifies: REQ-SCOPE-DISPOSAL
:verifies: REQ-OPAQUE-BODIES
:validates: SCN-SCOPE-DISPOSAL
:supersedes: VER-RUN-RECOVERY
:supersedes: VER-SQUASH-DIGEST
:supersedes: VER-WORKSPACE-CLEANUP

Promote an unclaimed wisp with relations and exact body bytes; compare ID, state, ancestry and relations before/after and confirm only persistence/location changes. Refuse promotion with a current claim and refuse source/graph loss during interrupted publication. Commit/merge the promoted material file through real Git and resolve its surviving host. Finalize completed rounds with and without summaries, preserving root state and exact supplied digest text; keep compact terminal identities/claim history while retained. Explicitly abandon unfinished execution without closing material items. Validate a recursive target set before deletion, refuse current claims and unintended material deletion, then process descendants before ancestors without handoff retention gates. A shared checkout remains needed by surviving scopes. Inject failures at promotion/cleanup publication boundaries and inspect bounded progress; retries must not sweep unrelated files or invent a successful completion.

Reject ordinary metadata edits/reopening and claims on retained ended-run wisps while permitting explicit unclaimed promotion with unchanged identity/body/state/relations. Endings retire idle sessions without prior retirement calls and refuse their current cross-run claims. Execution-only disposal preserves surviving material files and their dedicated hosting; fresh execution or explicit inheritance resumption does not move old wisps or remove a still-required material host.
:::

:::mara verification VER-BETA2-SURFACES
:mid: 01M439Z8VPXTEFZF8K3GHAP0JJ
:title: Verify the real beta.2 connected CLI and MCP workflow
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-BETA2-BOUNDARY
:verifies: REQ-CLI-MCP-PARITY
:verifies: REQ-CLI-JSON
:verifies: REQ-EXTERNAL-EXECUTION
:validates: SCN-SCOPED-EXECUTION
:validates: SCN-SCOPE-RELOCATION
:validates: SCN-SCOPE-DISPOSAL

Run equivalent disposable primary/linked repository workflows through real CLI/MCP processes and installed native/dispatcher packages. Exercise default initialization, inherited execution, own child run, named claim-next/owner mutations, retirement, cross-run pull handoffs, promotion, external Git merge plus relocation, optional-summary finalization, recursive disposal and external checkout-removal reporting if exposed. Compare domain outcomes and authoritative files, and test invalid-input no-write behavior, contention, missing/ambiguous authority and operation-specific interruptions. Check help/tools-list against implemented contracts, not a hard-coded beta.1 tool count; superseded workspace/binding, attach/detach, rebind and receiver-pruning operations must not pretend success. Confirm no migration/dual-layout engine, agent/process control, Git merge/worktree deletion, PR/CI action, or notification transport is executed. Local passing evidence is not publication/all-host/power-loss evidence.
:::
