# Work delivery backlog

These Git-tracked Work items coordinate delivery for this repository. Mara owns product contracts; the item files own task scope, dependencies, acceptance criteria, and progress. The [item format](../docs/item-format.mara.md) defines their storage contract.

## Release scopes

| Scope | Item | Status and purpose |
| --- | --- | --- |
| Repository delivery | [w-ee7bf41f](items/ee7bf41fb2e64323b9283a1716411e41.md) | Root aggregate for version scopes |
| `0.1.0-alpha.1` | [w-ce11f17d](items/ce11f17d361343b9bd1a9a6059aeb76d.md) | First usable CLI/MCP loop delivered and alpha published |
| `0.1.0-beta.1` | [w-abf475b5](items/abf475b5d09948deb3928341404d4fcb.md) | Implementation complete and beta.1 published; release evidence recorded |
| `0.1.0-beta.2` | [w-5b610f40](items/5b610f40e24341e58be7d8bd99ee4061.md) | Accepted scope-owned redesign; implementation and publication not yet delivered |

Alpha and beta.1 are published. The [beta.1 release item](items/e3640780b04f404bbb3baebfa3a50a38.md) is closed with publication evidence at final main revision `91a035d3773ed53e61f9757634299a2d7dfeff10`. Beta.2 is the separately selected next design/delivery target, with no dependency on beta.1 execution migration or publication. Aggregate items have `completion: children` and derive their status from their children; they are not claims or executor sessions.

### First alpha

| Order | Item | Purpose |
| --- | --- | --- |
| Implementation aggregate | [w-933a6d82](items/933a6d82674c4a23ab45e810b8319bd2.md) | Deliver the first usable Work item loop through CLI and MCP |
| 1 | [w-87b8795f](items/87b8795f934049c2acebaac42e81664d.md) | Establish the Rust core, Git discovery, and CI/release infrastructure |
| 2 | [w-0fac69ec](items/0fac69ec66004e088ce2b22cc0119bd2.md) | Load version-1 items and preserve their opaque bodies |
| 3 | [w-13ca14c6](items/13ca14c6e5c14b24a5dea551c343bbeb.md) | Evaluate relationships, effective completion, and readiness |
| 4 | [w-cec96d7e](items/cec96d7e174d47c1ab3117968b0bba0f.md) | Implement safe shared operations for durable items |
| 5 | [w-393f86fa](items/393f86fafaf54449944d27da1af0a24b.md) | Expose the durable item loop through CLI and structured JSON |
| 6 | [w-04368710](items/04368710988a405d8e96f85d3bae6b01.md) | Expose the same durable item loop through MCP |
| 7 | [w-e1121992](items/e1121992e0574b4b9b163b85e17ec601.md) | Verify bootstrap adoption and document the first usable loop |
| Release | [w-74c22e40](items/74c22e40cbc249229f86e355a663a942.md) | Prepare and verify the first alpha after the aggregate completes |

The seven implementation items are done after their merged PRs. The alpha release item is done with [publication evidence](items/74c22e40cbc249229f86e355a663a942.md). See [delivery conventions](../docs/delivery.mara.md) for the release process.

### First beta

| Item | Purpose |
| --- | --- |
| [w-fba5815b](items/fba5815b224f4c74a194ec2be6dd5d90.md) | Implementation aggregate |
| [w-661da1f2](items/661da1f2e8fd4ad99ddbe041ffe9e3b0.md) | Shared entity-file storage, locking, recovery and worktree-view reconciliation |
| [w-299765f4](items/299765f4605e414f974c2aa7cbe85b49.md) | Exclusive claims and recovery |
| [w-65c88091](items/65c8809176c3464b8bf341500e9ecb4c.md) | Scoped selection and atomic claim-next |
| [w-ef35536d](items/ef35536d06194c6b95c7e7b7fbd06aa0.md) | Parameterized template preview |
| [w-d8c51fea](items/d8c51fea0fb943a79a3cf057c34fd200.md) | File-backed runs and mixed material/wisp template expansion |
| [w-292b3f92](items/292b3f92dfbc4dbba7cd656a21ef7095.md) | Receiver-scoped handoffs |
| [w-6fba939e](items/6fba939e61b543b59adda11a829c238a.md) | Session and workspace associations |
| [w-7a9d4d85](items/7a9d4d85256f438ea22e098800959cda.md) | Explicit workspace cleanup |
| [w-350f6c4f](items/350f6c4f665f46dba64e36e73c585838.md) | Run squash with a digest or explicit ephemeral discard |
| [w-210453f0](items/210453f0dd104a978097120cb21bc733.md) | Complete CLI/MCP beta adoption verification |
| [w-e3640780](items/e3640780b04f404bbb3baebfa3a50a38.md) | Prepare, verify, and publish the first beta |

The beta.1 implementation aggregate and release item are complete. [Publication evidence](items/e3640780b04f404bbb3baebfa3a50a38.md) records the annotated tag, successful all-host build/publish/public-install workflow and public packages/release. The old [shared-store recovery follow-up](items/155dd1c73ef3412da9d2a44ec57dcfbc.md) is closed as cancelled/superseded by beta.2, not as an implemented fix. Historical task bodies remain intact. Work relationships determine actual readiness; table order is for navigation only.

### Second beta: scope-owned execution

The accepted `0.1.0-beta.2` redesign is recorded in [scope ownership](../docs/scopes.mara.md), [scoped execution](../docs/scoped-execution.mara.md), and [acceptance definitions](../docs/beta2-verification.mara.md). The published beta.1 binary does not implement it. This local scope's integrated baseline implements the verified 13-operation hosting/material-routing profile. The initialization issue branch adds an 18-operation subset including `init`/`status` and ordinary material close/reopen/repair; [the local-source guide](../docs/using-beta2-initialization.md) documents it. Initialization scope integration and downstream/combined acceptance remain pending. All template-specific operations, including pre-init material-only behavior, belong to runs/templates under the approved allocation. No beta.1 execution migration or compatibility layer is required; old delivery/history records remain intact.

| Item | Purpose | Prerequisites |
| --- | --- | --- |
| [w-16e2d0f5](items/16e2d0f58f0b4386b6d042bf1c8380b8.md) | Implementation aggregate | Derived from children |
| [w-3f7dc770](items/3f7dc77062a64fb285d95b172ea9808d.md) | Record accepted architecture, retirement and delivery contracts | Integrated and done |
| [w-10f481ed](items/10f481edd6d44c458e8c091aa76a5e7c.md) | Freeze exact envelopes and the complete CLI/MCP matrix | Integrated and done |
| [w-c0f0be8c](items/c0f0be8cb04340eb9e272d97b30095fb.md) | Onboarding/hosting contract amendment and restart boundary | Integrated and done |
| [w-7bbc0619](items/7bbc0619ab46461fa0d5c3b42edbeef1.md) | Restarted scope anchors, primary-root discovery and authoritative hosting | Integrated and done |
| [w-6bf6198a](items/6bf6198a34494a94a30a9ba53ed976a6.md) | Repository init/status, standing root run and housekeeping ignores | Hosting |
| [w-45f7bb7d](items/45f7bb7d2c9a43ea993cb7dc8ac8e89d.md) | Inherited/child runs, derived membership and template destinations | Initialization |
| [w-54de7594](items/54de759473154b008161327e9c961a81.md) | Immutable named shortcuts, retirement and exclusive local claims | Runs |
| [w-2cc1d6ac](items/2cc1d6acf22e4a48bbf9839998674264.md) | Sender-local addressed handoffs and pull inboxes | Sessions/claims |
| [w-16eaaab0](items/16eaaab0283b4bb894c3e202681ffc3a.md) | Validated relocation after external merge, preserving ownership | Sessions/claims |
| [w-fab94eea](items/fab94eea738c4bd593aa300c141dabfc.md) | Unclaimed wisp promotion with stable identity | Sessions/claims |
| [w-0e159a0e](items/0e159a0eb11145d9942f7366262fb4c2.md) | Optional-summary success, distinct abandonment and bottom-up disposal | Handoffs, relocation, promotion |
| [w-5ad29afa](items/5ad29afa1c6d4fedb953b8cd694e34f4.md) | Project-wide publication artifact cleanup and exceptional recovery | Lifecycle (transitively all runtime writer lanes) |
| [w-4334a9b9](items/4334a9b95506477d880e221673383514.md) | Real CLI/MCP/package acceptance and implemented usage/skill guidance | Lifecycle and publication cleanup |
| [w-3932a6c6](items/3932a6c6ff404fb0b7cdf0ace756c2a2.md) | Manual local-scope owner and eventual release follow-through | Coordination now; release phase requires implementation completion |

Selected delivery mode: **bounded local beta.2 scope**, owned by [w-3932a6c6](items/3932a6c6ff404fb0b7cdf0ace756c2a2.md#control-plane-handoff), on `feature/w-3932a6c6-beta2-scope`. The owner records included IDs, integration order, local acceptance and final remote/publication gates. It is manual; the version/implementation items remain derived aggregates.

Local `main` was fast-forwarded to architecture seed `3e23fa4`; the scope branch starts there. No remote push occurred. The architecture item is now closed after reviewed, verified scope integration; the wire task is also closed after reviewed, verified scope integration of its accepted specification. The onboarding/hosting amendment is integrated and done after fresh independent review and committed-revision verification. The fresh hosting restart is reviewed and verified in the scope, including the user-approved fixture-only integration remedy. Its original stopped implementation remains unaccepted and preserved. Initialization, including ordinary material close/reopen/repair, is now the next gate before runs/templates; readiness does not authorize dispatch. Readiness never authorizes launch. Do not replay the seed or require an architecture-only PR. Child issue branches start from the current scope tip, undergo acceptance and required local review, then squash-integrate and are verified before closing. One eventual scope PR carries the seed and issue history; rebase merge only with separate authority. Do not push local `main` independently.

Readiness is determined by the actual graph, not table order. Handoffs, relocation and promotion can proceed independently after ownership. The owner's former implementation dependency is now an explicit release-phase gate so coordination can start before implementation completes; all child dependencies remain unchanged. Release/candidate preparation and publication retain separate authority and evidence gates. Do not create live scope/run/session fixtures in this repository merely to test the planned model.

## Working with the backlog

Use the Work CLI or MCP against the selected checkout to inspect items, graph diagnostics, and readiness. A practical CLI entry point is `work --worktree PATH item ready`; run `work item --help` for available operations. Keep item IDs and opaque bodies intact when editing metadata.

For this repository, close an included beta.2 issue only after acceptance and required local review pass and its integrated result is verified on the declared scope branch. Individual issue delivery instead targets `main`; follow [the declared-target gates](../docs/delivery.mara.md#evidence-and-completion). The scope owner retains combined acceptance, cloud review/CI, remote-main integration and its release/publication evidence. Run destructive/state-changing scenarios on disposable copies, never on this live backlog. Work's generic item lifecycle is unchanged.

## Storage direction

`ADR-SCOPE-OWNED-EXECUTION` replaces beta.1's global execution placement and independent lifetimes with [scope-owned, versionable execution](../docs/scoped-execution.mara.md), Git-local hosting declarations, derived context and parent-controlled disposal. Preserve reusable safe-file/graph principles, but remove duplicate workspace registries, run member lists, rebinding and receiver-retention machinery. Exact new envelope/API contracts are now [accepted wire knowledge](../docs/beta2-wire.mara.md) after complete specification validation and independent local review. Architecture and original wire delivery are done after verified local scope integration. The onboarding/hosting amendment is freshly reviewed and verified in this local scope; its item records the actual integrated revision and the user-approved direct-integration exception. Historical wire or stopped-worker checks do not establish amendment/runtime acceptance. The bounded hosting runtime item is integrated/done; remaining beta.2 runtime, combined acceptance and release items remain open. The user separately approved closing the completed beta.1 release and cancelling the obsolete recovery follow-up.
