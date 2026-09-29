# Work delivery backlog

These Git-tracked Work items coordinate delivery for this repository. Mara owns product contracts; the item files own task scope, dependencies, acceptance criteria, and progress. The [item format](../docs/item-format.mara.md) defines their storage contract.

## Release scopes

| Scope | Item | Status and purpose |
| --- | --- | --- |
| Repository delivery | [w-ee7bf41f](items/ee7bf41fb2e64323b9283a1716411e41.md) | Root aggregate for version scopes |
| `0.1.0-alpha.1` | [w-ce11f17d](items/ce11f17d361343b9bd1a9a6059aeb76d.md) | First usable CLI/MCP loop complete; release preparation open |
| `0.1.0-beta.1` | [w-abf475b5](items/abf475b5d09948deb3928341404d4fcb.md) | Remaining accepted initial capabilities and a separate beta release item |

The version labels are planning targets. Each release item selects the actual publication version. Beta depends on the completed alpha scope, so the next globally ready item is alpha release preparation. Aggregate items have `completion: children` and derive their status from their children; they are not claims or executor sessions.

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

The seven implementation items are done after their merged PRs. The alpha release item remains open and depends on the implementation aggregate. See [delivery conventions](../docs/delivery.mara.md) for its evidence and publication requirements.

### First beta

| Item | Purpose |
| --- | --- |
| [w-fba5815b](items/fba5815b224f4c74a194ec2be6dd5d90.md) | Implementation aggregate |
| [w-661da1f2](items/661da1f2e8fd4ad99ddbe041ffe9e3b0.md) | Shared coordination store and worktree-view reconciliation |
| [w-299765f4](items/299765f4605e414f974c2aa7cbe85b49.md) | Exclusive claims and recovery |
| [w-65c88091](items/65c8809176c3464b8bf341500e9ecb4c.md) | Scoped selection and atomic claim-next |
| [w-ef35536d](items/ef35536d06194c6b95c7e7b7fbd06aa0.md) | Parameterized template preview |
| [w-d8c51fea](items/d8c51fea0fb943a79a3cf057c34fd200.md) | Recoverable file-backed runs |
| [w-292b3f92](items/292b3f92dfbc4dbba7cd656a21ef7095.md) | Receiver-scoped handoffs |
| [w-6fba939e](items/6fba939e61b543b59adda11a829c238a.md) | Session and workspace associations |
| [w-7a9d4d85](items/7a9d4d85256f438ea22e098800959cda.md) | Explicit workspace cleanup |
| [w-350f6c4f](items/350f6c4f665f46dba64e36e73c585838.md) | Durable run finalization digest |
| [w-210453f0](items/210453f0dd104a978097120cb21bc733.md) | Complete CLI/MCP beta adoption verification |
| [w-e3640780](items/e3640780b04f404bbb3baebfa3a50a38.md) | Prepare, verify, and publish the first beta |

The beta implementation items are children of the implementation aggregate. The beta release item is its sibling and depends on its completion. Work relationships determine actual readiness; table order is for navigation only. The open engineering contracts in [Mara](../docs/open-questions.mara.md) must be resolved during the relevant implementation items. Deferred product ideas are not part of this beta scope.

## Working with the backlog

Use the Work CLI or MCP against the selected checkout to inspect items, graph diagnostics, and readiness. A practical CLI entry point is `work --worktree PATH item ready`; run `work item --help` for available operations. Keep item IDs and opaque bodies intact when editing metadata.

For this repository, close an implementation item only after its acceptance criteria pass and its change is merged. Release items also require the publication evidence in their bodies. Run destructive or state-changing acceptance scenarios on disposable copies, never on this live backlog. Main remains the integration branch; implementation work starts in dedicated item branches.
