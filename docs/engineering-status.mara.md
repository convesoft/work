# Engineering status and risks

Beta.2's product scope is settled. This document records accepted choices, the separate engineering specification gate, and known risks.

## Accepted beta.2 direction

The product decisions are settled in [[ADR-SCOPE-OWNED-EXECUTION]], [scope ownership](scopes.mara.md), [scoped execution](scoped-execution.mara.md) and [acceptance definitions](beta2-verification.mara.md). Accepted knowledge is not implementation or passing evidence. The executable and [usage guide](using-work.md) still describe beta.1; retired contracts retain their identities and historical bodies.

| Settled point | Contract |
| --- | --- |
| Implicit item scopes; explicit boundary anchors; virtual primary-checkout root | [[REQ-SCOPE-TREE]], [[DES-SCOPE-RESOLUTION]] |
| Per-checkout Git-local authority, candidate diagnostics and explicit validated relocation | [[REQ-SCOPE-HOSTING]], [[DES-SCOPE-TRANSITIONS]] |
| Versionable local run files, no automatic ignore rule or global execution entity store | [[REQ-VERSIONED-EXECUTION]], [[DES-SCOPE-LAYOUT]] |
| Standing default root run, one home per run, nearest-run inheritance, derived membership | [[REQ-SCOPED-RUNS]], [[DES-SCOPED-RUNS]] |
| Immutable actor/namespace/ID shortcuts, active → retiring → retired, no name reuse/fallback | [[REQ-IMMUTABLE-SESSIONS]], [[DES-IMMUTABLE-SESSIONS]] |
| Exclusive claims, atomic name-based current ownership, optional acquisition-specific fencing | [[REQ-SCOPED-OWNERSHIP]], [[DES-SCOPED-CLAIMS]] |
| Sender-local addressed messages, qualified cross-run destinations and pull discovery | [[REQ-SENDER-HANDOFFS]], [[DES-SENDER-HANDOFFS]] |
| Stable-ID unclaimed promotion; optional-summary finalization; bottom-up explicit disposal | [[REQ-WISP-PROMOTION]], [[REQ-SCOPE-DISPOSAL]], [[DES-SCOPE-TRANSITIONS]] |
| No migration/compatibility machinery; redesigned CLI with MCP parity; Git/agents external | [[REQ-BETA2-BOUNDARY]], [[DES-BETA2-SURFACES]] |

## Confirmed lifecycle and routing details

The scope review resolved the following observable choices. There are no remaining product-level questions identified by this review; the engineering specification gate below remains open.

| Confirmed choice | Contract |
| --- | --- |
| Ended child boundaries do not silently resume ancestor execution; fresh execution or inheritance resumption is explicit | [[DES-SCOPED-RUNS]] |
| Retained ended-run wisps allow inspection, unclaimed promotion and disposal, not ordinary edits, reopening, new claims or handoffs | [[REQ-SCOPE-DISPOSAL]], [[DES-SCOPE-TRANSITIONS]] |
| Root rotation requires explicit abandonment of unfinished root wisps, not completion of the material repository backlog | [[REQ-SCOPED-RUNS]] |
| Ending retires idle names automatically after their current claims in every run drain | [[DES-IMMUTABLE-SESSIONS]] |
| Execution disposal preserves hosting needed by material work; relocation precedes external checkout removal | [[REQ-SCOPE-HOSTING]], [[DES-SCOPE-TRANSITIONS]] |
| Physical-copy inspection is read-only; Work mutations use authoritative routing | [[DES-SCOPE-RESOLUTION]] |
| Child-run templates declare scope parentage explicitly; no invented edges or storage-only membership | [[DES-SCOPED-RUNS]], [[DES-TEMPLATE-FORMAT]] |
| Current wisps cannot be reparented into a different effective run; promote first, rather than adding transfer machinery | [[REQ-SCOPED-OWNERSHIP]], [[DES-SCOPED-RUNS]] |
| Item-addressed context follows stable item identity; run/session destinations remain exact immutable identities | [[DES-SENDER-HANDOFFS]] |
| Unqualified named claim-next selects within one resolved run namespace; qualified identity can support broader selection | [[DES-SCOPED-CLAIMS]] |

These choices and their rationale are recorded in [[ADR-SCOPE-OWNED-EXECUTION]] with repeatable checks in [beta.2 verification](beta2-verification.mara.md). Open risk treatments below are not unanswered scope decisions or claims of eliminated risk.

## Specification gate before behavior implementation

[[DES-BETA2-WIRE]] remains deliberately **draft**. The [w-10f481ed](../.work/items/10f481edd6d44c458e8c091aa76a5e7c.md) item must freeze exact root/scope/child-reference identifiers and envelopes, per-checkout host declarations, run phases and root rotation, named/qualified identity inputs, local claim authorization/endings, handoff/inbox schemas, narrow interrupted relocation/promotion/disposal continuation and the full CLI/MCP/error matrix. These are engineering details constrained by accepted behavior, not permission to restore superseded machinery.

If freezing those formats reveals a new behavioral choice, clarify it and update canonical knowledge before implementation. Do not silently accept illustrative file fields or command spellings. No root run/scope fixture is created in this live repository by documentation authoring.

## Delivery and evidence

[The beta.2 backlog](../.work/README.md#second-beta-scope-owned-execution) separates architecture, wire specification, bounded implementation, connected adoption and release preparation. Implementation completion follows [the declared-target delivery gates](delivery.mara.md#evidence-and-completion). Release preparation separately requires intentional version/changelog work, a final main revision, all declared supported-host/package smokes, actual external protection/trust verification and explicit publication authority.

Retirement of a beta.1 contract does not erase its delivery or release history. No beta.1 execution migration/dual runtime is required. Keep existing version-1 material item identity/body and reusable graph/template semantics unless explicitly revised. Work never launches/stops agents or executes physical Git/worktree actions; external quiescence assertions remain cooperative caller facts, not authenticated process control.

## Deliberately later work

TUI, push/watch notifications, receipts/acknowledgements, mandatory handoff archival, backup/restore, a generic transaction/recovery service, arbitrary remote/multi-host coordination, outcome classification, provider polling, shared-resource locks and PR/CI automation are not beta.2 prerequisites. Per-operation partial progress and diagnostics remain necessary; no new generic orchestration service or persistent global index is introduced.

## Material risks

:::mara risk RISK-DIVERGENT-VIEWS
:mid: 01M3KAW7AX5F7AF33GXDF9955F
:title: Resolve the correct worktree without mixing item versions
:status: accepted
:treatment: open
:affects: REQ-DURABLE-FILES
:affects: DES-SHARED-FILES
:affects: REQ-SCOPE-HOSTING

Beta.2 exposure: versioned scope/run copies can exist in several checkouts after merges, but only explicit Git-local hosting establishes authority. Resolve an item's entire file from its nearest host, starting at the primary root, and report missing/duplicate hosts rather than choosing stale or newest content. Promotion/relocation must not lose surviving material routing. Current obligations are [[REQ-SCOPE-HOSTING]], [[REQ-VERSIONED-EXECUTION]] and [[REQ-WISP-PROMOTION]]. The historical association wording below describes beta.1, not a fallback rule for beta.2.

Historical beta.1 risk description:

Linked worktrees can hold different versions of the same material item. A global definition cache keyed only by item ID can return stale main content or combine state from one worktree with a body from another. Resolve the item's associated worktree before loading its whole file, and expose that source to the caller. With no association, use the selected checkout. Keep explicit branch-local inspection available; do not treat an unavailable associated worktree as permission to substitute stale content.

Material mutations are saved in that worktree immediately. Run finalization/disposal must neither reset them nor lose the source association needed to find them. Reassociation and external workspace cleanup retain their existing explicit lifecycles. Obligation: [[REQ-WORKTREE-VIEWS]].
:::

:::mara risk RISK-FILE-DB-CONSISTENCY
:mid: 01M3KAW7B7E9QCWDXAHNGK897V
:title: Interrupted multi-file updates can lose or duplicate work
:status: accepted
:treatment: tolerated
:affects: DES-PERSISTENCE-BOUNDARY
:affects: DES-EPHEMERAL-LAYOUT
:affects: DES-SCOPE-TRANSITIONS

Beta.2 exposure: local scope files and nonversioned hosting declarations have separate publication boundaries. Relocation, promotion, root rotation and bottom-up disposal need bounded inspectable progress, not a generic transaction engine. This risk remains tolerated for per-file template publication under [[REQ-RUN-ATOMICITY]]; [[DES-SCOPE-TRANSITIONS]] constrains retention, source checks and retry behavior. Historical beta.1 descriptions below remain provenance, not a shared-store requirement.

Historical beta.1 risk description:

Separate entity files have separate atomicity boundaries. A crash during template expansion can leave only some items or relationships. For beta this is an accepted limitation: validate before writing, report known created items on a returned error, leave surviving files inspectable, and let the agent remove partial work and retry. No application record, automatic rollback or idempotency machinery is required. Ordinary invalid-graph diagnostics remain applicable. Other operations must preserve their specific obligations: outgoing context before release and root digest before squash cleanup. Explicit discard intentionally permits deletion without a digest while preserving material work and rejecting unresolved outside references. [[DES-FILE-COORDINATION]], [[REQ-RUN-ATOMICITY]] and [[REQ-RUN-FINALIZATION]] define these boundaries. The stable risk ID is retained from the earlier file/database design.
:::

:::mara risk RISK-STALE-COORDINATION
:mid: 01M3KAW7BG8DYZMDFABEMN939V
:title: Lost or stale ownership can allow duplicate execution
:status: accepted
:treatment: open
:affects: REQ-SIDE-STATE
:affects: ADR-INITIAL-SCOPE
:affects: REQ-SCOPED-OWNERSHIP

Beta.2 exposure: an unexpected checkout loss, obsolete copied claim files, or a raw Git rewrite can make local ownership uncertain. Scope routing plus repository-wide locking and immutable named identities protect cooperating calls, not arbitrary filesystem tampering or process control. Report uncertainty; do not activate merged copies, expire claims or replace named identities implicitly. Current ownership/relocation obligations are [[REQ-SCOPED-OWNERSHIP]] and [[REQ-SCOPE-HOSTING]]. Historical beta.1 generation/recreation wording below is not a beta.2 migration requirement.

Historical beta.1 risk description:

A crashed agent, stale claim, or manual operational-file deletion or storage reset can leave an old executor working while a new agent claims the same item. Preserved item bodies do not reconstruct lost claim files. A plain-file layout remains vulnerable to edits outside the shared lock. Define claim identity, fencing of former owners, recovery behavior, and operator-visible uncertainty. Work cannot stop an external agent merely by changing its recorded claim. Proposed obligations: [[REQ-CLAIM-EXCLUSION]] and [[REQ-CLAIM-RECOVERY]].
:::
