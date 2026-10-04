# Beta.2: scope ownership and routing

This is accepted product direction for the planned `0.1.0-beta.2`, not implemented behavior or release evidence. [Scoped execution](scoped-execution.mara.md) defines the companion run/session/message contracts; [acceptance definitions](beta2-verification.mara.md) describe future checks. Historical beta.1 contracts retain their identities but are not requirements for beta.2 where explicitly superseded. The current executable and [usage guide](using-work.md) still describe beta.1.

## Vocabulary and obligations

:::mara term TERM-SCOPE
:mid: 01M439VEMN0BSAJ1EV8MKB7PAN
:title: Scope
:status: accepted

An item-rooted logical ownership boundary within one Git repository. Ordinary item ancestry provides implicit scopes; create a stored scope anchor only when an item establishes its own run or checkout-hosting boundary. Parentless material items attach to the virtual repository root in the resolved scope model without rewriting their item headers. Scope ancestry is a tree; item dependencies and informational links remain a graph. Scope identity survives changes of hosting checkout. A workspace is a Git checkout, not an independently identified Work entity.
:::

:::mara term TERM-SCOPE-HOST
:mid: 01M439VENVSQ6R2KZE87EHKNRT
:title: Scope host
:status: accepted

The one authoritative Git working checkout hosting a stored scope anchor and its local records. Hosting is machine-local routing, separate from the versionable scope content. Many scopes may share a checkout, and a scope may explicitly relocate between checkouts. Copies created by Git commit/merge do not become competing live hosts merely by existing. The virtual root is always hosted by Git's primary checkout, regardless of its checked-out branch.
:::

:::mara term TERM-EFFECTIVE-RUN
:mid: 01M439VEQ114XVY598ARM3J32A
:title: Effective run
:status: accepted

The execution context selected by the nearest established run boundary at an item's scope or an ancestor scope, falling back to the standing default run at the virtual root after initialization only when no nearer boundary overrides it. A retained ended boundary blocks fresh execution until the caller explicitly starts a new run or resumes inheritance; it does not silently fall through to an ancestor. A dedicated checkout alone does not establish a new run. Each run has one owning scope and one storage home; child scopes using an inherited run do not partition or duplicate its records. A nearer run establishes a distinct execution context and named-session namespace. Retained terminal wisps stay owned by their original run rather than joining later rounds.
:::

:::mara requirement REQ-SCOPE-TREE
:mid: 01M439VEYD0Y41C9FNBJ3KCXMT
:title: Resolve inherited item scopes beneath a virtual root
:status: accepted
:kind: functional
:derives_from: SCN-SCOPED-EXECUTION

Work shall derive implicit scope ancestry from item parentage and expose a virtual repository root for parentless items without storing synthetic parent assertions in their files. The virtual root is a non-executable scope anchor, not an obligation waiting for the whole repository to finish. Stored scope anchors exist only at explicit run or hosting boundaries. Descendants inherit hosting and effective execution context until overridden. Cross-scope dependencies retain ordinary graph semantics, and implicit ancestry shall not duplicate item state.
:::

:::mara requirement REQ-SCOPE-HOSTING
:mid: 01M439VEZNS64G19K6NBBSXWXC
:title: Locate authoritative scope content through Git-local hosting
:status: accepted
:kind: quality
:derives_from: SCN-SCOPE-RELOCATION
:supersedes: REQ-WORKTREE-VIEWS

Work shall locate the virtual root in Git's primary checkout, irrespective of branch name, and resolve other scope hosts through Git's worktree inventory and per-checkout Git-local declarations. Exactly one authoritative host may serve a live stored scope. Missing hosts/files and multiple hosts shall produce diagnostics and explicit candidates where available, not newest-copy selection or stale fallback. An explicit relocate/adopt operation shall validate the destination and transfer routing for the scope and its continuing runs; no per-item rebind is required. Scope identities and content shall remain separate from checkout identity and machine-local paths.

Explicit physical-checkout inspection shall be read-only and shall not activate a copy; Work mutations use authoritative scope routing. Disposing execution shall preserve a dedicated hosting boundary while surviving material items still require that checkout. Material relocation is a separate validated operation before external checkout removal.
:::

:::mara requirement REQ-VERSIONED-EXECUTION
:mid: 01M439VF0WBD8FK7VXVP790TYP
:title: Keep scope-owned execution content safe to commit and merge
:status: accepted
:kind: constraint
:derives_from: SCN-SCOPE-RELOCATION
:supersedes: REQ-SIDE-STATE
:supersedes: REQ-EPHEMERAL-FILES

Work shall store inspectable run manifests, wisps, named sessions, claims and sender-owned handoffs under the owning scope's hosting checkout. Execution content is eligible for Git versioning, shall not be automatically ignored, and shall survive ordinary commit/merge as scope-identified content. Committing a wisp does not promote it; merging a copy does not establish live authority. Durable items, templates and digest extensions retain their .work directories. Only machine-local routing/coordination belongs outside versioned content; there shall be no duplicate repository-wide execution store or required persistent lookup index.
:::

:::mara requirement REQ-SCOPED-RUNS
:mid: 01M439VF246Y6B1KM8ZKD9HSYQ
:title: Derive execution context and membership from scopes
:status: accepted
:kind: functional
:derives_from: SCN-SCOPED-EXECUTION
:supersedes: REQ-SINGLE-RUN

After explicit initialization, Work shall provide a standing current run at the virtual root. Reads shall not initialize missing state. The standing run need not await repository-wide task completion; explicit rotation replaces it with a fresh default run. An item-rooted run covers derived descendant execution until a nearer child run takes over, with no attach/detach member registry. Each run has one owning scope/storage home. At most one current run is established at any scope boundary. Successful non-root run finalization shall require required member obligations done, no relevant current claims, and descendant runs ended; its root item lifecycle stays independent.

An ended non-root run boundary shall remain explicit: material execution does not silently fall back to an ancestor. Further execution requires an explicit fresh run or resumption of inherited execution, without moving retained wisps out of their original run. Root rotation shall refuse unfinished root-run wisps unless the caller explicitly chooses abandonment; it still requires no current claims and ended descendant runs, but does not require repository-wide material completion.
:::

:::mara requirement REQ-IMMUTABLE-SESSIONS
:mid: 01M439VF3CV7YA4D13KAN7QM7Q
:title: Use immutable run-owned identity shortcuts with retirement
:status: accepted
:kind: functional
:derives_from: SCN-SESSION-DRAIN

Named sessions shall provide a case-sensitive run-local readable name plus one immutable actor/namespace/external-ID tuple. An identical repeated registration may be unchanged; different identity under the same name is a conflict. A run shall never rebind or reuse a retired name. Session states are active, retiring and retired: retiring blocks new claims but permits existing owner operations, and retirement completes only after current claims end. Availability observations are not lifecycle authority. Unqualified lookup uses only the effective run; another run requires an explicit qualified reference. Work shall not start, stop or resume external sessions.
:::

:::mara requirement REQ-SCOPED-OWNERSHIP
:mid: 01M439VF4MPG17PBENARVEX8TW
:title: Tie exclusive claims to stable sessions and effective runs
:status: accepted
:kind: functional
:derives_from: SCN-SESSION-DRAIN
:supersedes: REQ-CLAIM-RECOVERY

Claims shall remain repository-wide exclusive per item and have immutable acquisition IDs and explicit endings. A named-session shortcut shall supply identity without repeated flags; mixed named/explicit identity inputs shall be rejected. Without a named session, accept the explicit fields required by that operation. Named owner mutations may atomically resolve the current claim; an explicit claim ID remains available for acquisition-specific stale protection. Claims retain the resolved identity/session instance and do not follow another owner. Reparenting or introducing a nested run shall refuse changes to claimed items' effective contexts. Hosting relocation may preserve claims only with destination validation and caller quiescence attestation; Work does not quiesce agents. No timer, restart, inferred cache rebuild or retirement request silently releases claims.

For claim-next with an unqualified session name, Work shall resolve one namespace from an explicit run or the selected scope, and consider only items with that effective run. It shall not resolve the same readable name separately in competing candidate runs. Explicitly qualified named identity may be used for broader selection, subject to ordinary eligibility and ownership checks.

Reparenting a current wisp shall refuse a change into a different effective run rather than transferring its storage/ownership implicitly. Reparenting within its run remains available under ordinary graph and claim safety. Useful temporary work that needs cross-run reparenting must first be explicitly promoted to material.
:::

:::mara requirement REQ-SENDER-HANDOFFS
:mid: 01M439VF5V9AR1ER0ZXZ6QPA70
:title: Discover addressed handoffs with sender-owned lifetime
:status: accepted
:kind: functional
:derives_from: SCN-SENDER-HANDOFF
:supersedes: REQ-HANDOFF-CONTEXT

Work shall store each handoff and its opaque body in its sending run, including reports addressed to the parent. Destinations may be items, immutable named sessions or runs, with explicit qualification for cross-run targets. Inspection/acquisition and filtered inbox/list expose incoming context through resolved scope locations. Handoffs do not affect readiness. No notification transport, receipt, acknowledgement, mandatory upward transfer, unread-state archive or receiver-completion disposal gate is required. The sender's run/scope may dispose its handoffs regardless of recipient state; missing context after disposal is not proof of delivery.

Item-addressed messages shall follow the item's stable identity across material relocation, promotion and later execution rounds until sender disposal. Run/session destinations shall remain bound to exact immutable identities, with no forwarding into replacement rounds. An ended sending run shall admit no new handoffs; final reports must be published before ending it.
:::

:::mara requirement REQ-WISP-PROMOTION
:mid: 01M439VF73D7XVDBE1QTJTT92Q
:title: Promote an unclaimed wisp without replacing its identity
:status: accepted
:kind: functional
:derives_from: SCN-SCOPE-DISPOSAL

Work shall provide explicit promotion of an unclaimed wisp into a material item, retaining its full item ID, recorded state, opaque body and relationships. Promotion changes persistence/location, not completion or work ancestry. Validate the resulting source/graph and refuse active claims during the move; surviving graph references shall remain resolvable. Material files reach surviving checkouts through external Git commit/merge and scope routing, not implicit content synchronization. Committing execution files alone is not promotion.
:::

:::mara requirement REQ-SCOPE-DISPOSAL
:mid: 01M439VF8BPK1D7PQMD3R0R0JX
:title: Finalize or abandon execution through parent-controlled lifetimes
:status: accepted
:kind: quality
:derives_from: SCN-SCOPE-DISPOSAL
:supersedes: REQ-RUN-FINALIZATION
:supersedes: REQ-WORKSPACE-CLEANUP

Completed runs may finalize with or without a supplied digest; any supplied digest preserves exact text without rewriting or closing the root. Unfinished abandonment is explicitly distinct and shall not complete material obligations. Keep compact terminal session identities and claim history until explicit disposal. A parent may dispose a bounded execution subtree in one operation, validating first and processing descendants before ancestors. Refuse current claims, unintended material-file deletion and structural references whose deletion would invalidate surviving material work; do not impose handoff-read, receiver-state or archival gates. Preserve actionable partial-progress evidence after interruption. Disposal of one scope shall not remove a checkout hosting another required scope; physical worktree/Git deletion remains external.

Ending a run shall retire its idle named sessions as part of the operation, without requiring separate prior retirement calls. Current claims of those named identities in any run shall block ending; no ending releases claims implicitly. Retained ended execution shall permit historical inspection, explicit unclaimed-wisp promotion and explicit disposal, but not ordinary wisp edits/reopening, new acquisitions or new handoffs. Material lifecycle and editing remain independent. Execution disposal shall not remove hosting still required for surviving material work.
:::

:::mara requirement REQ-BETA2-BOUNDARY
:mid: 01M439VF9N4ZZJZ41ZP4HMKVN5
:title: Replace obsolete bookkeeping without legacy migration machinery
:status: accepted
:kind: constraint
:derives_from: SCN-SCOPED-EXECUTION

Beta.2 shall replace independent Work workspace records/item bindings, explicit run attach/detach, named-session rebinding, global execution entities and receiver-retention handoff pruning with the scope-owned model. There is no beta.1 execution migration, compatibility alias obligation or dual-layout runtime. Existing version-1 material items and reusable template semantics remain supported unless explicitly changed. The CLI command layout may be restructured, but CLI/MCP share core behavior and typed outcomes. Exact file/wire versions must be explicit; obsolete execution files shall not be silently reinterpreted as new state. Agent execution, external Git/worktree actions, PR/CI actions, push notifications, remote coordination and a generic recovery service remain outside this increment.
:::
