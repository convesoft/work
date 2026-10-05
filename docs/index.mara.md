# Work product knowledge

Work is a local, agent-first issue tracker implemented in Rust. Durable item/graph semantics stay small; external tools execute agents and Git/PR/CI actions. The next accepted increment is **`0.1.0-beta.2`: scope-owned execution**. This is a product contract, not implemented behavior or release evidence.

## Reading this corpus

`accepted` means the product decision is settled, not that code exists or tests passed. `draft` is an explicit specification gate. `retired` preserves historical knowledge and references without imposing it on the replacement. Mara items describe the product; `.work/items/` contains delivery tasks and acceptance/progress records.

For beta.2, begin with [[ADR-SCOPE-OWNED-EXECUTION]], [scope ownership](scopes.mara.md), [scoped execution](scoped-execution.mara.md), and [acceptance definitions](beta2-verification.mara.md). [[DES-BETA2-WIRE]] now freezes the accepted [wire envelopes and complete CLI/MCP operation/error matrix](beta2-wire.mara.md), with [closed schemas](schemas/beta2-wire-v2.schema.json). This is reviewed knowledge, not implemented beta.2 behavior or verified delivery integration. The current binary and [usage guide](using-work.md) still expose beta.1. [Repository onboarding](onboarding.mara.md) specifies planned top-level init/status, housekeeping-only ignores and independent pre-init material planning; the onboarding/hosting contract amendment requires fresh review/integration before the restarted foundation proceeds.

## Corpus map

| Document | Purpose |
| --- | --- |
| [Scope ownership](scopes.mara.md) | Current beta.2 vocabulary, scope/host/run inheritance and observable obligations |
| [Scoped execution](scoped-execution.mara.md) | Current beta.2 placement, session/claim/message lifecycles, promotion and disposal; accepted wire specification |
| [Beta.2 acceptance](beta2-verification.mara.md) | Real connected scenarios and repeatable verification definitions, not passing evidence |
| [Product](product.mara.md) | Goals, participant roles, and shared vocabulary |
| [Scenarios](scenarios.mara.md) | Reusable planning/graph flows and retired beta.1 context flows |
| [Requirements](requirements.mara.md) | Reusable obligations and explicitly retired beta.1 requirements |
| [Design](design.mara.md) | Reusable item/graph/opaque-body principles and retired execution architecture |
| [Item format](item-format.mara.md) | Preserved version-1 material item contract and frontmatter schema |
| [Template format](template-format.mara.md) | Preserved rendering vocabulary; scope-aware publication is defined by beta.2 |
| [Historical file storage](storage.mara.md) | Retired shared entity placement and storage foundation, retained for history |
| [Historical execution IO](execution-api.mara.md) | Retired beta.1 source/binding and authorization interface |
| [Historical claims](claims.mara.md) | Retired beta.1 exact claim API |
| [Historical runs](runs.mara.md) | Retired explicit membership and publication API |
| [Historical execution context](execution-context.mara.md) | Retired workspace registry, rebindable sessions and receiver-retained handoffs |
| [Historical finalization](run-finalization.mara.md) | Retired digest-required and receiver-gated cleanup API |
| [Decisions](decisions.mara.md) | Rationale and explicit supersession relationships |
| [Verification](verification.mara.md) | Reusable item/graph checks and retired beta.1 execution checks |
| [Engineering status](engineering-status.mara.md) | Settled beta.2 scope, specification gate and material risks |
| [Delivery](delivery.mara.md) | Knowledge/task/evidence ownership, branches and protected release preparation |

## Agreed initial scope

The enduring boundary from [[ADR-INITIAL-SCOPE]] remains: Work tracks items, prerequisites, reusable templates, temporary execution and ownership through CLI/MCP. It does not execute agents, create/remove physical worktrees, merge branches, perform PR/CI actions, or interpret review outcomes. The TUI, notification transport, remote coordination and general backup/recovery services remain outside beta.2.

The beta.2 direction replaces the former shared execution lifetime choices; it does not require migration or compatibility for beta.1 execution files. Existing version-1 material items, opaque bodies, template rendering, priorities and ordinary relationships remain useful contracts. Do not erase historical IDs, MIDs or delivery evidence merely because the next implementation removes an operation.

## Beta.2 ownership model

```text
Git repository
└─ Virtual root scope — primary checkout, any branch
   └─ Standing default run
      └─ Item tree — implicit inherited scopes
         └─ Explicit scope anchor when needed
            ├─ Dedicated checkout hosting, optionally
            └─ Own run, optionally
               ├─ Wisps
               ├─ Immutable named sessions
               ├─ Claims
               └─ Sender-owned addressed handoffs
```

Scopes own stable logical identity, ancestry and local content. Git supplies checkout facts; small Git-local declarations establish one authoritative host per stored live scope. Parent-local child references plus Git worktree discovery replace a global scope/workspace registry. Reads resolve complete authoritative files, including uncommitted state, and expose missing/ambiguous source diagnostics rather than selecting stale or newest copies.

Runs are execution rounds attached to scopes. Descendants inherit the nearest run until establishing their own. A checkout override alone does not create a run or partition its records: each run has one storage home. Membership is derived, not managed by attach/detach. The virtual root is not an executable obligation; its standing run is rotated explicitly and does not wait for the entire repository to finish.

| State | Beta.2 authority and lifetime |
| --- | --- |
| Material items, templates and digests | Git-versionable `.work/items/`, `.work/templates/`, `.work/digests/` |
| Stored scope anchors and child references | Versionable scope content at explicit boundaries; ordinary ancestry stays implicit |
| Runs, wisps, named sessions, claims and handoffs | `.work/scopes/<scope-id>/runs/<run-id>/` in the owning scope's authoritative host |
| Hosting declarations | Per-checkout actual Git metadata, not tracked paths or a duplicate global entity registry |
| Coordination | Short repository-wide locking plus source checks for cooperating callers; no required daemon/database |
| Graph, inbox and location views | Disposable in-memory views from authoritative records |

Committing execution detail does not promote wisps. Merging a scope copy does not duplicate live authority. Explicit validated relocation after an external merge can continue the same run/session/claim identities in a parent checkout; keeping current claims requires caller executor-quiescence attestation. Several scopes may share a checkout. Physical removal remains external.

Named sessions are immutable run-owned actor/namespace/external-ID shortcuts. Unqualified names resolve only within the effective run; another run requires qualification. Their lifecycle is active → retiring → retired, blocking new assignments while claims drain. Names are never rebound or reused in the same run. Claims stay repository-wide exclusive per item; owner operations may resolve current ownership through the named identity or explicitly fence an acquisition by claim ID.

Handoffs are addressed opaque messages stored in the sending run, even when addressed to the parent or a sibling run. Incoming context is discovered by reads/acquisition/inbox calls, not notifications. Sender disposal has no acknowledgement, receiver-completion or archival obligation. A parent decides what to read, promote or retain.

Promotion preserves an unclaimed item's identity/body/state/relationships while making it material. Completed runs may finalize with an optional summary without closing their roots; abandonment is distinct. Compact terminal identities/claim history remain while retained. Explicit recursive scope disposal validates its target set and operates bottom-up, refusing active claims and unintended material loss, not unread handoffs. [[DES-SCOPE-TRANSITIONS]] defines the accepted boundaries; narrow partial/retry formats are frozen by [the accepted wire specification](beta2-wire.mara.md).

## Agreed relationship model

[[DES-RELATION-MODEL]] defines `parent` and `depends_on` lifecycle edges plus informational `related` and `discovered_from`. Children are required work, not mutually exclusive alternatives. Aggregates derive completion; manual parents have their own final execution step after children finish. Explicit ancestor prerequisites gate descendants; internal waits for children do not. Cross-scope graph edges preserve these semantics. [[DES-LIFECYCLE]] retains explicit closure/cancellation/reopening without cascading manual state.

## Provenance

[[ADR-SCOPE-OWNED-EXECUTION]] supersedes [[ADR-FILE-STATE]]'s global execution placement and independent retention machinery. The original plain-file, local coordination, opaque-body and graph principles remain. Conflicting beta.1 requirements/designs/checks are retired with replacement links; they remain inspectable history, not a migration obligation. The beta.2 delivery plan is in [the backlog](../.work/README.md#second-beta-scope-owned-execution). The selected bounded local delivery scope is owned by [w-3932a6c6](../.work/items/3932a6c6ff404fb0b7cdf0ace756c2a2.md#control-plane-handoff), with included issues targeting `feature/w-3932a6c6-beta2-scope`. This Git/delivery convention does not initialize Work execution scopes or waive the wire, review, integration or publication gates.

## Implementation evidence

The published `0.1.0-beta.1` prerelease implements the durable/coordination surface, including the former workspace, membership, session-rebinding and receiver-retention operations. [Its completed release item](../.work/items/e3640780b04f404bbb3baebfa3a50a38.md) records publication at final main revision `91a035d3773ed53e61f9757634299a2d7dfeff10`; the old recovery follow-up is cancelled as superseded, not fixed. [README](../README.md), [the usage guide](using-work.md), and existing delivery items distinguish source capability, local checks and publication evidence. This documentation change adds no runtime implementation, version bump, changelog, passing execution evidence, tag or publication.

## Execution implementation contracts

Beta.2 execution work starts by accepting [[DES-BETA2-WIRE]], then follows bounded hosting, runs/templates, sessions/claims, handoffs, relocation, promotion, lifecycle and real adoption items. Architecture knowledge, implementation acceptance and protected release preparation are separate. Implementation completion follows [the declared-target delivery gates](delivery.mara.md#evidence-and-completion). The release item separately requires one verified final main revision, supported-host packages, external protection/trust settings and explicit publication authority.
