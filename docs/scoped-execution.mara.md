# Beta.2: scope-owned execution

Scopes define ownership and hosting; runs define execution rounds and session namespaces. The contracts below are accepted beta.2 behavior, not implementation evidence. The accepted [wire specification](beta2-wire.mara.md), referenced by DES-BETA2-WIRE below, freezes serialized envelopes and the command/tool/error matrix after specification checks and independent review. Knowledge acceptance is not runtime implementation or verified delivery integration. Git merges, external session/process control, and physical worktree creation/removal remain external.

## Replacement contracts

:::mara design DES-SCOPE-RESOLUTION
:mid: 01M439Z8X02NXGZ61B60XJHR2D
:title: Resolve item scopes and per-checkout authoritative hosting
:status: accepted
:kind: structure
:satisfies: REQ-SCOPE-TREE
:satisfies: REQ-SCOPE-HOSTING
:supersedes: DES-SHARED-FILES
:supersedes: DES-WORKSPACE-ASSOCIATIONS

## Ownership and discovery

Derive ordinary scopes from item ancestry, using a virtual non-executable root for parentless items without serializing synthetic item parent edges. Store an immutable anchor only for explicit run or checkout-hosting boundaries. Parent scopes reference child anchors; do not maintain a competing global scope registry. Scope containment is a tree, while ordinary dependencies/related/provenance retain their current graph semantics.

Git supplies the primary checkout and linked-worktree inventory. The primary checkout hosts the virtual root on any branch. Each checkout's actual Git metadata directory holds its small machine-local hosting declaration; never assume its .git entry is a directory. Scan declarations into a disposable scope-to-host map under repository coordination. Several scopes may share a checkout. A missing host, duplicate declaration, missing source or invalid anchor is diagnostic, not permission to fall back to another copy. Human callers may choose among reported candidates; JSON/MCP expose the same candidates and require a subsequent explicit authority selection.

## Material source selection

Resolve an item's whole material file through the nearest scope host, including uncommitted bytes. A dedicated child hosting boundary overrides inherited location for its subtree. Resolve identities and graph references across sibling scopes without copying their state. Keep explicit read-only physical checkout inspection available without activating the inspected copy. Work mutations always use the authoritative resolver; external Git/manual edits are not Work authority changes. Reparent/context changes must maintain scope-tree consistency and refuse affected current claims; a host relocation does not change run ownership. Exact scope-anchor fields and host-file grammar are specified by [[DES-BETA2-WIRE]] and [the complete wire candidate](beta2-wire.mara.md).

## Material boundary roots

Only material items may establish stored run or dedicated hosting boundaries. The virtual repository root is the sole non-item exception. A wisp must first be explicitly promoted before establishing either boundary; ordinary wisp parentage still derives implicit scopes. This wire-review clarification preserves one-home wisp ownership and avoids coupling a disposable run to a nested anchor rooted in its temporary work.
:::

:::mara design DES-SCOPE-LAYOUT
:mid: 01M439Z8YB71WJAXH41B0DRQ08
:title: Place versionable execution files beneath stable scope anchors
:status: accepted
:kind: data
:satisfies: REQ-VERSIONED-EXECUTION
:satisfies: REQ-BETA2-BOUNDARY
:supersedes: DES-PERSISTENCE-BOUNDARY
:supersedes: DES-ENTITY-LIFECYCLES
:supersedes: DES-EPHEMERAL-LAYOUT
:supersedes: DES-STORAGE-API
:supersedes: DES-STORE-FOUNDATION

Keep existing material items at .work/items/<item-id>.md, templates at .work/templates/<name>.yaml and optional root digests at .work/digests/<root-id>/<run-id>.md. Place execution content under .work/scopes/<scope-id>/runs/<run-id>/ with run.yaml, items/<item-id>.md, sessions/<session-record-id>.yaml, claims/<claim-id>.yaml plus matching endings, and handoffs/<handoff-id>.md. Scope anchors and parent-local child references are versionable; machine-local checkout paths, hosting declarations and the shared coordination lock are not copied into tracked scope content. The virtual root has no material item document; its stable `root` anchor/path representation is specified by [[DES-BETA2-WIRE]].

Do not create an automatic .gitignore rule for execution records. Optional commits/merges preserve them, but temporary/material classification follows ownership and explicit promotion, not Git tracking. A branch copy cannot activate a scope or resurrect ownership merely by existing. Authority follows hosting declarations and validated run state. There is one storage home for an inherited run: a child with only a dedicated checkout still uses the parent's run directory, and must establish its own run for independent local run records. No shared global claims/runs/handoff/workspace entity directories or authoritative aggregate index remain. These are the required ownership boundaries; [[DES-BETA2-WIRE]] supplies the complete decoded envelopes and lifetime/version rules.
:::

:::mara design DES-SCOPED-RUNS
:mid: 01M439Z8ZNZJREQM5EMV53HG47
:title: Use standing root execution and nested run boundaries
:status: accepted
:kind: structure
:satisfies: REQ-SCOPED-RUNS
:satisfies: REQ-SCOPE-TREE
:satisfies: REQ-GRAPH-TEMPLATES
:supersedes: DES-RUN-API
:supersedes: DES-TEMPLATE-RUNS

Explicit initialization establishes the virtual root's standing default run. Ordinary durable CRUD and material-only template planning do not require execution initialization; reads do not create it. Root execution is always available after healthy initialization. It does not mean every repository obligation must complete. Explicit rotation retires the current root round under ownership/child-run safety rules and establishes a fresh default identity; partial rotation must be inspectable rather than silently returning healthy execution.

A run attaches to an item scope and inherits the owning host. Child scopes inherit the nearest run until establishing one of their own, independently of checkout overrides. Derive membership/context from current ancestry instead of material member lists and attach/detach operations. A root item has context in its own run, but completing that item and finalizing its round are separate. The run root's own recorded completion is excluded from the member-finished calculation, even though it has execution context in that run. For a non-root run, finished requires nonempty required member work resolved by the existing graph semantics and no relevant claims; unfinished child-run work cannot be hidden by a narrower context. Successful finalization additionally requires child runs ended and session ownership drained. Parent material obligations still account for child item outcomes across run boundaries.

Material-only expansion follows scope-resolved destination selection. Wisp expansion targets an effective run, including the initialized root run when inherited; preserve version-1/version-2 template rendering, explicit prospective graph validation and per-file partial publication. Exact start/rotate/finalize calls and derived membership result fields are specified by [[DES-BETA2-WIRE]].

## Ended boundaries and retained work

Finalized or abandoned non-root runs retain their execution boundary. Material descendants cannot silently switch to an ancestor namespace: the caller explicitly starts a fresh run or resumes inherited execution. Such a context change refuses affected current claims and preserves any dedicated material host. Retained wisps remain owned by their original run and do not become members of a later round merely because their parentage is unchanged. They remain available for graph/reference inspection, explicit unclaimed promotion or disposal, not ordinary edits, reopening or new claims. Material CRUD remains independent of ending execution.

Root rotation requires no current ownership, ended descendant runs and drained named identities, including their claims in other runs. Unfinished root-run wisps require an explicit abandon-and-rotate choice; ordinary rotation refuses them. Ending never marks such wisps or material obligations done, moves wisps into the new run, or deletes retained history.

## Template ancestry

Validate each prospective wisp's parent chain against its target effective run. A child-run expansion must explicitly connect wisps beneath that run's scope through template/existing parent edges; reject mismatches before publication. Parentless wisps are valid in the virtual-root run without synthetic item parents. Do not add parent edges automatically or substitute storage placement for ancestry-derived current membership. Preview exposes the actual declared graph and destinations.

Apply the same run-placement validation to later wisp reparenting, not only template publication. Refuse a current wisp's cross-effective-run parent change; do not introduce implicit or explicit wisp-transfer machinery in beta.2. Reparent within the same run under ordinary graph/claim constraints, or promote first and then change material ancestry.

Only material items may establish an item-rooted run boundary; promote a wisp first. The virtual-root default run remains the non-item exception. This does not restrict ordinary temporary descendants or add automatic parent edges.
:::

:::mara design DES-IMMUTABLE-SESSIONS
:mid: 01M439Z90ZJW2GX13H0HKPVDV8
:title: Register immutable names and drain their claims
:status: accepted
:kind: structure
:satisfies: REQ-IMMUTABLE-SESSIONS
:satisfies: REQ-SCOPED-OWNERSHIP
:supersedes: DES-CLAIM-CONTEXT

A run-owned named session stores a generated record identity, readable case-sensitive name, immutable actor label and immutable external namespace/ID. The name is a lookup shortcut, not an authenticated OS principal or process handle. Registration with the identical tuple may return unchanged; another tuple under the same name conflicts. Do not rebind, rename into a used name, or reuse a retired name in that run. A replacement uses a new name or a new run.

Lifecycle is active -> retiring -> retired. Retirement and acquisition serialize under the shared lock. Retiring refuses new ownership while still allowing matching owner completion, release and outgoing handoffs. When no current claims reference the named identity, including qualified uses in other runs, complete retirement. Keep compact terminal identity for inspection until owning execution is explicitly disposed. Availability observations do not create lifecycle transitions; no heartbeat/expiry or external session control is introduced.

Unqualified --session NAME resolves only within the item's effective run. A different run requires explicit qualification, never ancestor fallback. Scope creation alone does not establish a name namespace. Reads reveal both lifecycle and supplied external observations. Exact registration/retirement/inbox spellings and qualified-reference syntax are specified by [[DES-BETA2-WIRE]].

Successful finalization, explicit abandonment and root rotation retire idle active/retiring names as part of ending the owning run. They first refuse any current claim referencing those named record identities anywhere in the repository. Separate prior retirement calls are unnecessary; terminal identity/history remains inspectable. Ended runs cannot register names, acquire ownership or originate further handoffs.
:::

:::mara design DES-SCOPED-CLAIMS
:mid: 01M439Z929GHZ4CMGYSYZ8ZGH0
:title: Resolve named ownership atomically with scope-local claim records
:status: accepted
:kind: structure
:satisfies: REQ-SCOPED-OWNERSHIP
:satisfies: REQ-CLAIM-EXCLUSION
:satisfies: REQ-WORK-SELECTION
:supersedes: DES-CLAIM-API
:supersedes: DES-EXECUTION-IO
:supersedes: DES-FILE-COORDINATION

Each acquisition and its separate ending live in the effective run's one storage home. Cross-host lookup scans authoritative scope/run files under a short repository-wide coordination lock; claim exclusion remains repository-wide per item, never per branch, scope or session. Keep canonical item IDs and existing priority-then-ID selection with filters that do not hide outside prerequisites. No eligible work differs from corruption, missing source, ambiguous authority or lock contention.

Named identity input resolves actor/namespace/external ID plus immutable record identity under the same held lock as eligibility and acquisition. Explicit identity remains available with the fields actually needed by the operation. Reject mixed forms. Acquisitions retain identity snapshots and exact IDs; no rebind or timer transfers ownership. For owner mutation, a readable named session may resolve the matching current claim atomically. If an explicit claim ID is supplied, validate that exact acquisition and reject stale pairs. Thus a name-only request intentionally addresses current matching ownership, whereas an ID-qualified request fences a particular acquisition.

Complete with existing bounded ordering: save supplied outgoing handoffs, persist item completion, then publish claim ending. Keep partial publication visible and do not resubmit already-saved messages blindly. Reparenting/nested-run creation cannot change a claimed item's effective context. Hosting relocation preserves the same scope/run/session/claim identities only after destination validation and caller assertion that affected executors are paused/stopped. Explicit recovery/reassignment still requires stopped-executor assertions. Shared lock plus source fingerprint checks protect cooperating callers, not concurrent raw editors or external Git rewrites.

Unqualified named claim-next resolves exactly one run namespace from an explicit run or selected scope before candidate selection. Its candidate set includes only items with that effective run, intersected with ordinary filters; outside prerequisites remain visible. It never looks up a possibly different identity under the same name after choosing among multiple runs. An explicitly qualified session identity may own eligible work across runs. Ended-run wisps are historical, not executable candidates; expose their terminal execution restriction rather than presenting them as newly claimable.
:::

:::mara design DES-SENDER-HANDOFFS
:mid: 01M439Z93MFGXD0HMS7P7NVTG9
:title: Keep addressed context at the sender and discover it through reads
:status: accepted
:kind: structure
:satisfies: REQ-SENDER-HANDOFFS
:satisfies: REQ-OPAQUE-BODIES
:supersedes: DES-HANDOFF-RECORDS

Each handoff is a separate Markdown/frontmatter document in the sending run's handoffs/ directory. Its body is caller-authored opaque UTF-8 context. Metadata distinguishes sender context from intended item, immutable named-session or run destinations, including the parent orchestrator and explicitly qualified sibling scopes. A destination changes lookup, not physical storage, readiness or lifetime. Parent-addressed final reports remain in the child run.

Derive incoming lookup through the scope/run resolver without a second authoritative message registry. Item inspection and acquisition expose incoming addressed messages; filtered inbox/list covers session/run destinations. Work does not push to agents or require a watcher, daemon, receipt, unread counter, acknowledgement, deduplication service or automatic parent forwarding. External orchestrators choose when to read and dispatch context. An executor that makes no further Work calls cannot be promised spontaneous awareness.

Messages disappear when their sender's owning execution is explicitly disposed, regardless of recipient state or whether the parent read them. No receiver-retention pruning contract remains. Preserve explicit diagnostics for unavailable/malformed source context rather than pretending it is an empty successful lookup. Exact metadata endpoint encoding and inbox filters are specified by [[DES-BETA2-WIRE]].

An item destination is the canonical stable item ID. Explicit cross-run qualification validates the address when authored, but does not confine an item-addressed message to that recipient round: inspection follows the same item through promotion, hosting relocation and later material execution rounds while the sender is retained. Run and named-session destinations instead retain exact immutable run/record identities; they do not follow replacements. Preserve original sender/run provenance so older context is distinguishable. Publish final reports before ending the sending run; terminal execution cannot originate new messages.
:::

:::mara design DES-SCOPE-TRANSITIONS
:mid: 01M439Z94YJ1F6Q6WTEWS785S1
:title: Relocate hosts, promote work and dispose execution bottom-up
:status: accepted
:kind: structure
:satisfies: REQ-SCOPE-HOSTING
:satisfies: REQ-WISP-PROMOTION
:satisfies: REQ-SCOPE-DISPOSAL
:supersedes: DES-SQUASH-DIGEST
:supersedes: DES-FINALIZATION-API
:supersedes: DES-CONTEXT-API

## Relocation

Git commit/merge is external. Work's explicit relocate/adopt operation validates a destination containing the continuing scope/run content, checks source/host fingerprints and destination consistency, requires quiescence attestation when retaining claims, and changes machine-local hosting. Do not reset current claim IDs or session identities merely because the host changes. A merged copy alone is not authority. Report ambiguous candidates for explicit selection and preserve bounded progress if host updates stop partway; no hidden newest-copy rule or generic transaction engine.

## Promotion

Move an unclaimed wisp to its scope-resolved .work/items/ destination with the same canonical identity and exact state/body/relationships. Validate source/destination and the prospective graph before publication. Preserve relationships only while their targets remain resolvable; do not silently erase a surviving dependency when later disposing temporary targets. Once material, ordinary Git merge carries the work into a surviving checkout, and hosting must select that surviving source. Stable-ID publication across locations requires narrow inspectable partial/retry handling, not a new migration service.

## Ending and disposal

Successful finalization checks required work, current ownership and descendant-run/session safety. Every ending refuses current claims of its named identities in other runs as well as relevant local claims, then retires idle named sessions without separate prior retirement. An optional digest is written as an extension, never a new item or root-body rewrite; no digest is required when omitted. Retain compact terminal session identities and acquisition/endings while run files remain. Ended execution is historical: allow inspection, explicit unclaimed-wisp promotion and disposal, not ordinary wisp mutation/reopening, new acquisition or new handoffs. An ended non-root boundary remains until explicit fresh execution or resumption of inheritance; neither silently transfers old wisps. Explicit abandonment is distinct from success and does not close material obligations. The parent may explicitly dispose a bounded subtree: prevalidate the target set, refuse active claims and unintended durable deletion, then execute descendant-first. No handoff-read/receiver/archival proof is required. Deleting structural targets still cannot silently leave surviving material graph references invalid.

Execution disposal, material-host relocation and physical checkout removal are separate. Preserve scope anchors/hosting still required by surviving material items even after all execution there is disposed; a hosting-only boundary remains valid. Before removing that checkout, the caller must externally retain/merge material content and explicitly validate relocation to its surviving host. A checkout hosting another required scope cannot be removed. External tools perform removal after Work's safety/routing transition; report failure/partial state for continuation, without a mandatory independent workspace record or invented Git-retention proof. Exact terminal phases, bounded progress/results and source-transfer schemas are specified by [[DES-BETA2-WIRE]].

Wire-review safety clarification: when a former authoritative source is unavailable, explicit adoption or authority selection must refuse until it is recovered and can be compared with the destination. A stopped-executor assertion or fingerprint of a plausible copy cannot establish continuity with unreadable old content. Work does not supply a generic lost-source recovery service.
:::

:::mara design DES-BETA2-SURFACES
:mid: 01M439Z96AJH0V8WPC2PK9ZKSR
:title: Keep scope operations shared across redesigned CLI and MCP
:status: accepted
:kind: interface
:satisfies: REQ-BETA2-BOUNDARY
:satisfies: REQ-CLI-MCP-PARITY
:satisfies: REQ-CLI-JSON
:satisfies: REQ-EXTERNAL-EXECUTION
:supersedes: DES-CLI-JSON
:supersedes: DES-MCP-STDIO

Replace obsolete workspace registry/binding, explicit run membership, session rebinding, global entity storage and receiver-retention pruning rather than providing migration/compatibility aliases. Ordinary item/relationship and reusable template operations preserve their model; command grouping may change. CLI and MCP use the same core operations, structured outcomes, source/conflict rules and invalid-input no-write behavior. Existing JSON success/error envelope semantics remain a reusable transport contract; do not imply the beta.1 advertised command/tool count is beta.2's surface.

Expose scope inspection/hosting/relocation/disposal, run inspection/start/explicit resumption of inherited execution/root rotation/finalization/abandonment, immutable named-session registration/retirement, identity shortcuts on every relevant consumer, and pull inbox operations. Shared reads identify owning scope, effective run, source checkout/path and contextual diagnostics. APIs must let machine callers resolve host ambiguity explicitly, without requiring interactive TUI input. Retained historical content may be physically inspected without activating it. Physical-copy mode is read-only; all Work mutations use authoritative routing. Ended-run restrictions, stable item-addressed context, one-namespace unqualified named claim-next and material hosting after execution disposal have equivalent CLI/MCP outcomes. Require explicit initialization for new execution state; reject obsolete or unsupported execution formats instead of auto-conversion. Neither target version selection nor accepted contracts authorize release publication or external actions.
:::

:::mara design DES-BETA2-WIRE
:mid: 01M439Z97QQ2N76958306N0TJR
:title: Freeze beta.2 file envelopes and operation matrix before implementation
:status: accepted
:kind: interface
:refines: DES-BETA2-SURFACES

The accepted normative beta.2 wire specification is [the wire specification](beta2-wire.mara.md), with [closed decoded envelope/input/result schemas and the exhaustive operation/error catalog](schemas/beta2-wire-v2.schema.json). These are part of this design, not illustrative snippets. The complete candidate passed specification consistency, closed-schema validation, complete Mara schema/project validation and independent local review before knowledge acceptance. Acceptance records selected knowledge, not implemented behavior, passing runtime/package checks or scope integration. The delivery item remains open until its separate control-plane acceptance and verified scope-integration gates pass.

The specification freezes version-2 root/scope anchors and parent-local child references, dedicated Git-local hosting overrides and shared coordination; one-home active/finalized/abandoned runs, standing-root rotation, explicit ended barriers and inheritance resumption; immutable run-local actor/namespace/external-ID names with qualified selectors and cross-run draining; immutable scope-local claim acquisition/endings with repository-wide exclusion, atomic current-owner resolution and optional acquisition fencing; sender-local addressed frontmatter/body, stable item versus immutable run/session destinations and pull context; and narrow fingerprinted start/tree-change, ending/rotation, host-transfer, stable-ID promotion and bottom-up disposal continuation.

The exhaustive matrix includes preserved material CRUD/repair/graph/selection/templates, redesigned execution consumers, all required/optional identity forms, no-null/unknown-input rules, structured result/error shapes, initialization/read-only physical diagnostics, bounded close/release/reassignment publication and invalid-input no-write semantics. Superseded workspace/binding, attach/detach, rebinding, receiver-pruning, global execution and storage-generation recovery operations are removed, not aliased. No migration, automatic ignore, global index, process/Git action or generic recovery service is added.

The confirmed scope-review semantics remain binding: ended boundaries need explicit fresh execution/inheritance resumption even after terminal disposal; historical wisps are inspectable/promotable/disposable, not editable/reopenable/claimable; unfinished root wisps require explicit abandonment at rotation; ending retires idle names only after all cross-run claims drain; material hosting survives execution disposal; physical copies are read-only; child-run ancestry is explicit, cross-run wisp reparenting refuses, item messages follow stable identity and unqualified named claim-next selects one namespace. Wire-review user clarifications restrict stored boundary roots to material items and require recovery/comparison of unavailable old sources before adoption.

Run the specification consistency checker and complete Mara schema/project validation, inspect affected links/status guidance and Work diagnostics, then obtain the independent local review required by the assigned Work item. The future behavioral/package checks remain [beta.2 acceptance definitions](beta2-verification.mara.md); none is claimed passing merely because these schemas validate.
:::
