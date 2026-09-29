# Product requirements

Each accepted obligation has a repeatable verification definition. Draft obligations express candidate behavior and must not silently become implementation requirements.

:::mara requirement REQ-LOCAL-STATE
:mid: 01M3KAQ0RMASYMS5MFZT2V24PM
:title: Store local project work
:status: accepted
:kind: functional
:derives_from: SCN-PLAN-WORK

Work shall support creating, reading, and updating local work items and their recorded state across process sessions. Descriptions and expected outcomes shall remain available to subsequent sessions without dependence on chat history.
:::

:::mara requirement REQ-WORK-GRAPH
:mid: 01M3KAQ0RV586F8NX3WB3KBANF
:title: Represent dependencies between work items
:status: accepted
:kind: functional
:derives_from: SCN-NEXT-WORK

Work shall represent blocking relationships between identifiable work items so that a dependent item can be distinguished from its prerequisites.
:::

:::mara requirement REQ-ACTIONABLE-WORK
:mid: 01M3KAQ0S2058DZ5PV5APC4HF8
:title: Derive actionable work from prerequisites
:status: accepted
:kind: functional
:derives_from: SCN-NEXT-WORK

Work shall expose which work items are actionable from their own state and satisfaction of lifecycle prerequisites. An item with an unsatisfied explicit or inherited prerequisite shall not be presented as actionable. Manual parents shall wait for their children before becoming executable; aggregates shall never be executable. Child waits shall not propagate back down as blockers. Closing, including explicit cancellation with a reason, resolves the obligation. Reopening recalculates eligibility without automatically reopening completed manual parents or dependents. Invalid selected graphs shall prevent readiness and claim operations as defined by [[REQ-GRAPH-INTEGRITY]].
:::

:::mara requirement REQ-GRAPH-TEMPLATES
:mid: 01M3KAQ0S98JMTNQB7NZMN52VE
:title: Instantiate repeatable work as graph items
:status: accepted
:kind: functional
:derives_from: SCN-REPEATABLE-REVIEW

Work shall support reusable templates that instantiate work items and relationships for a repeatable process. The instantiated activities and gates shall be inspectable and evaluated through the same work-item and dependency model used for manually created work.

Templates shall support caller-supplied parameters and a preview of the rendered items and relationships before publication. Preview shall not publish work or acquire claims. Initial template rendering may produce Markdown bodies; later Work operations shall treat those bodies as opaque content.
:::

:::mara requirement REQ-EPHEMERAL-WORK
:mid: 01M3KAQ0SGRTZFC0QZDH5A0KB1
:title: Support temporary graph work
:status: accepted
:kind: functional
:derives_from: SCN-QUIET-EXECUTION

Work shall support temporary operational work items that participate in dependency evaluation without requiring their lifecycle detail to be committed as durable project history.
:::

:::mara requirement REQ-DURABLE-FILES
:mid: 01M3KAQ0SR7XBK6HJM5TABWTN4
:title: Keep durable project truth in YAML and Markdown
:status: accepted
:kind: constraint
:derives_from: SCN-PLAN-WORK

Durable project intent and retained outcomes shall be stored in inspectable YAML and Markdown suitable for version control. A side database shall not be the sole authoritative location of durable project information.
:::

:::mara requirement REQ-SIDE-STATE
:mid: 01M3KAQ0SZWK4DWJ2H7F3B57ET
:title: Persist operational and maintenance state separately
:status: accepted
:kind: constraint
:derives_from: SCN-QUIET-EXECUTION

Work shall support a side database for operational and maintenance state alongside durable YAML and Markdown. Routine operational changes shall not require changes to version-controlled durable content.
:::

:::mara requirement REQ-CLAIM-EXCLUSION
:mid: 01M3KAQ0T6J4XY42QNMESJPZX6
:title: Atomically exclude competing claims
:status: accepted
:kind: functional
:derives_from: SCN-CONCURRENT-AGENTS

For the same item identity within one Git repository and its linked worktrees, simultaneous claim requests shall produce at most one active owner. Changing the worktree or run context shall not permit a second executor of that item. A losing requester shall receive an explicit conflict with inspectable ownership information. Other callers may read the item and its progress. Readiness listing alone shall not reserve work; claiming must recheck eligibility and ownership. Distinct items may have distinct owners concurrently.
:::

:::mara requirement REQ-WORKTREE-VIEWS
:mid: 01M3KAQ0TDAY6227Q9RFQZMJK1
:title: Keep durable worktree views isolated
:status: accepted
:kind: quality
:derives_from: SCN-CONCURRENT-AGENTS

A query shall use the durable file view of its selected worktree while sharing the intended repository coordination state. Indexing an item in one worktree shall not replace another worktree's divergent version in that other worktree's queries. Shared coordination must not make branch-local completion appear durably recorded in other branches.

The control session shall be able to inspect another worktree by selecting its filesystem path, including uncommitted item changes there, without changing its own checkout. The selected path determines durable file content; the resolved Git common directory determines shared coordination and temporary-work storage. Branch content isolation does not create independent claim identities.
:::

:::mara requirement REQ-CLAIM-RECOVERY
:mid: 01M3KAQ0TNDBXRJDVQE7R5XN7K
:title: Make stale ownership recoverable
:status: accepted
:kind: functional
:derives_from: SCN-CONCURRENT-AGENTS

Claims shall expose their owner and recorded timestamps. The initial version shall not automatically expire claims or require heartbeats. A claim persists until completion, explicit release, or explicit recovery/reassignment. The controller resolves an abandoned claim explicitly; an old owner's token shall no longer authorize owner operations after release or reassignment. Database-loss recovery is an explicit exceptional operation: stop existing executors, reconstruct file-derived state, and establish fresh claims rather than silently restoring or inferring ownership.
:::

:::mara requirement REQ-INDEX-REBUILD
:mid: 01M3KAQ0TWGHT4CWF6D8ZMZ166
:title: Rebuild derived indexes without destroying runtime state
:status: draft
:kind: quality
:derives_from: SCN-REBUILD-INDEX

Work shall rebuild derived graph and lookup data from durable and retained ephemeral files without rewriting those files or deleting existing claims and runtime records as an incidental side effect. Whole-database loss shall be reported as loss of operational state; it shall not discard surviving ephemeral files or describe lost coordination as a lossless cache refresh.
:::

:::mara requirement REQ-RUN-FINALIZATION
:mid: 01M3KAQ0V4YF7739TV39PW1EQK
:title: Preserve retained results before temporary cleanup
:status: accepted
:kind: quality
:derives_from: SCN-QUIET-EXECUTION

Squashing completed temporary work shall create a durable completed digest item from a caller-supplied summary before removing temporary work. Work shall preserve the supplied summary verbatim and retain the durable root with its existing completion policy. Interruption and retry shall neither duplicate the retained result nor erase it. Cleanup shall not leave unresolved references in surviving work or discard handoffs whose receivers still need them. Refuse cleanup while surviving outside references require temporary items, and identify those references for caller resolution. Publish retained results in a designated surviving checkout. Recovery mechanics remain engineering specifications; arbitrary promotion and unfinished-run discard are outside the defined squash operation.
:::

:::mara requirement REQ-GRAPH-INTEGRITY
:mid: 01M3KAQ0VCMP0CZV8YZ5JFQDP1
:title: Reject ambiguous or cyclic blocking graphs
:status: accepted
:kind: quality
:derives_from: SCN-NEXT-WORK

Work shall diagnose duplicate identities, unresolved relationship targets, malformed source, and invalid lifecycle graphs. [[REQ-RELATION-STRUCTURE]] defines single-parent, hierarchy-cycle, and combined-deadlock constraints. An invalid selected project/worktree graph shall prevent readiness and claim operations for that view, including claim-next; Work shall not attempt to prove that disconnected components are safe to schedule in the initial version. Inspection, diagnostics, and repair shall remain available, and unrelated projects shall continue normally. A rejected structured graph mutation shall preserve the previous graph; a repair operation may correct invalid source.
:::

:::mara requirement REQ-CLI-JSON
:mid: 01M3KAQ0VKJQ3GYNS1Q4CNC2QZ
:title: Expose a machine-readable CLI
:status: accepted
:kind: functional
:derives_from: SCN-PLAN-WORK

Core work operations shall be available through a CLI with structured JSON output and machine-distinguishable failures. Agents shall be able to inspect, create, relate, claim, update, and complete work, and operate templates and temporary runs, without parsing terminal presentation. [[DES-CLI-JSON]] defines the first durable-item command and JSON slice; coordination, template, and run commands require their separate interface contracts.
:::

:::mara requirement REQ-RUN-ATOMICITY
:mid: 01M3KAQ0VT4XC3ZYT94DJQ4ZJN
:title: Instantiate complete temporary runs
:status: draft
:kind: quality
:derives_from: SCN-REPEATABLE-REVIEW

A template instantiation shall validate its inputs and graph before publishing its ephemeral item files as a complete runnable run. Invalid input or interruption before publication shall not expose a partial runnable process. Retried instantiation semantics and durable template expansion remain open. Because the files are authoritative, this requires a file publication and reconciliation protocol, not only a SQLite transaction.
:::

:::mara requirement REQ-GATE-CONTEXT
:mid: 01M3KAQ0W24KKNVW4WJAJ7ZXFA
:title: Bind external completion to the intended subject
:status: retired
:kind: quality
:derives_from: SCN-REPEATABLE-REVIEW

Retired proposal: Work would interpret structured external observations and enforce revision-bound gate outcomes. The user rejected introducing an outcome engine. External executors check review/CI conditions and express remaining work as ordinary items and relationships, completing an item only after creating required blockers. They may write revision or result context as opaque narrative without Work parsing or classifying it.
:::

:::mara requirement REQ-EPHEMERAL-FILES
:mid: 01M3KATNBMTK05HMRGN2VF8TZD
:title: Keep temporary work authoritative in files
:status: accepted
:kind: constraint
:derives_from: SCN-QUIET-EXECUTION

Ephemeral item content and its graph relationships shall be stored in inspectable filesystem documents until explicitly squashed or cleaned. SQLite may index these files and coordinate ownership, but it shall not be the sole authoritative representation of temporary work.
:::

:::mara requirement REQ-CLI-MCP-PARITY
:mid: 01M3KATNBW0Y6XXNEGV39PMT8E
:title: Expose shared core operations through CLI and MCP
:status: accepted
:kind: functional
:derives_from: SCN-PLAN-WORK

The initial release shall expose the same core work operations through CLI and MCP. Equivalent requests against equivalent state shall use the same validation, selection, mutation, and coordination semantics, with equivalent structured results and failure meanings. Transport-specific envelopes may differ.
:::

:::mara requirement REQ-EXTERNAL-EXECUTION
:mid: 01M3KATNC4GWS8HFSRD0TW8AQ3
:title: Keep execution in external tools
:status: accepted
:kind: constraint
:derives_from: SCN-REPEATABLE-REVIEW

The initial release shall track work state and coordinate ownership without launching agents, creating or managing Git worktrees, or performing pull-request, merge, or CI actions. External tools perform those actions and may supply workspace context and observed outcomes to Work. A ready item is a request for activity, not automatic execution authorization.
:::

:::mara requirement REQ-PARENT-COMPLETION
:mid: 01M3KFAJ3MB5S0VWV1AWRT6HRW
:title: Require children before parent completion
:status: accepted
:kind: functional
:derives_from: SCN-NESTED-DELIVERY

A child shall represent required work within its parent. With `completion: children`, a parent shall have no execution of its own and shall derive done only when it has at least one child and every direct child is done. Empty aggregates shall remain open; adding or reopening unfinished children shall make an aggregate incomplete again. With `completion: manual` (the default), all children shall finish before the parent's own remaining work becomes executable and its completion is explicitly recorded. Manual items without children shall follow ordinary execution.
:::

:::mara requirement REQ-HIERARCHICAL-READINESS
:mid: 01M3KFAJ40FMZE2YK5EZ6DDHXC
:title: Combine hierarchy and prerequisites without deadlock
:status: accepted
:kind: functional
:derives_from: SCN-NESTED-DELIVERY

An item shall wait for the complete outcome of each `depends_on` target, including the target's children and any manual parent execution. Explicit prerequisites of ancestors shall also gate descendant execution. A parent's wait for its own children shall not propagate to those children. Siblings shall have no execution order unless lifecycle constraints explicitly establish one.
:::

:::mara requirement REQ-INFORMATIONAL-RELATIONS
:mid: 01M3KFAJ4BNP88DJFKAARNMV3Z
:title: Keep association and discovery independent of lifecycle
:status: accepted
:kind: functional
:derives_from: SCN-PLAN-WORK

Work shall support symmetric `related` associations and directed `discovered_from` provenance from discovered work to its origin item. Neither relation shall affect readiness, required scope, execution order, or completion. Discovery provenance shall be able to coexist with parentage or a dependency between the same items.
:::

:::mara requirement REQ-RELATION-STRUCTURE
:mid: 01M3KFAJ4PAZ9Y8DGK6EMVTFYE
:title: Validate lifecycle graph structure
:status: accepted
:kind: functional
:derives_from: SCN-NEXT-WORK

Each work item shall have at most one parent. Work shall reject hierarchy cycles and dependency combinations that cause lifecycle deadlocks, including cycles formed by mixing hierarchy, inherited prerequisites, and dependencies. Each semantic relationship shall be stored once, with children and blocks derived as reverse views of parent and depends_on. Related associations shall have symmetric navigation without duplicate authored edges.
:::

:::mara requirement REQ-HANDOFF-CONTEXT
:mid: 01M3KPWZ26S5T2HS2Z3R14XB3W
:title: Retain and deliver context to receiving work
:status: accepted
:kind: functional
:derives_from: SCN-HANDOFF

Work shall support caller-authored handoff context from one or more source items to one or more receiving items, including continuation where the same item appears on both sides. Getting or claiming a receiving item shall expose its incoming handoffs. Completion of source items shall not delete context still needed by receivers. Automatic cleanup shall become eligible only after all receiving obligations are closed/resolved; release, reassignment, and failed completion shall retain it. Explicit cancellation resolves a receiver for retention purposes. Reopening after cleanup does not restore deleted handoffs; callers use retained durable results and new context. Handoff references shall not alter readiness or add lifecycle dependencies.
:::

:::mara requirement REQ-OPAQUE-BODIES
:mid: 01M3KPWZ2JM0X6GM2CR3K7YHYA
:title: Preserve Markdown bodies as caller-authored content
:status: accepted
:kind: constraint
:derives_from: SCN-PLAN-WORK

Work shall treat Markdown bodies as opaque caller-authored text. Acceptance criteria, reproduction steps, and narrative results belong in the body. Except for initial template rendering, Work shall not parse body sections, inspect checklists, lint missing content, generate summaries, or insert or rewrite narrative sections. Metadata updates shall preserve existing body bytes. Inspection and context retrieval shall return the body verbatim; storing an explicitly supplied handoff or digest shall preserve supplied content.
:::

:::mara requirement REQ-WORK-SELECTION
:mid: 01M3KPWZ2YY3NGZZS1KXN0K47H
:title: Filter and prioritize work selection
:status: accepted
:kind: functional
:derives_from: SCN-NEXT-WORK

Work shall support priority and scoped filtering when listing or selecting eligible work. Claim-next shall select and reserve one eligible item in a single atomic coordination operation, applying the requested scope and priority ordering with a stable tie break. Concurrent callers shall not acquire the same coordination identity. No eligible work shall have an explicit empty outcome. Version-1 item priority is an integer from 0 (urgent) to 4 (backlog), default 2; canonical ID lexicographic order breaks equal-priority ties. The filter operation schema remains an interface design detail. Coordination identity is repository-wide per item across linked worktrees.
:::

:::mara requirement REQ-GRAPH-INSPECTION
:mid: 01M3KPWZ3ANRX5Z86TGQ9SND8Z
:title: Inspect relations and explain readiness
:status: accepted
:kind: functional
:derives_from: SCN-NEXT-WORK

Work shall expose graph relationships and explain item readiness using the same lifecycle evaluation as selection. Explanations shall identify unsatisfied explicit or inherited prerequisites, unfinished children, non-executable aggregates, and ownership restrictions where applicable. Explanations are structured observations of the evaluated state, not reservations or model-generated reasoning.
:::

:::mara requirement REQ-EXECUTOR-HINTS
:mid: 01M3KPWZ3PAP8M8Z21CSHNMVX5
:title: Allow optional model and thinking metadata
:status: accepted
:kind: functional
:derives_from: SCN-PLAN-WORK

Items shall allow optional model and thinking fields in machine-readable metadata to assist external assignment. Omitting either shall be valid. Work records the requested hints without launching a model or requiring a built-in provider/model catalog.
:::

:::mara requirement REQ-WORKSPACE-CLEANUP
:mid: 01M3KTFNTD8D5NW6V5NF5YQS0D
:title: Track workspace removal through an explicit cleanup item
:status: accepted
:kind: functional
:derives_from: SCN-WORKSPACE-CLEANUP

Workspace cleanup shall be represented by an ordinary explicit work item. Completing another item, releasing a claim, removing a named session, or finalizing a run shall not implicitly delete a worktree. Cleanup shall wait until no active execution or unfinished work still requires the target workspace and necessary commits, results, and handoff context have been retained. External tooling removes the actual worktree and reports success; Work then removes its operational workspace record and associations. Failed removal shall retain inspectable workspace context for retry. Shared users outside the originating run shall also be considered. Work shall not execute filesystem or Git worktree removal itself.
:::

:::mara requirement REQ-SINGLE-RUN
:mid: 01M3KVG3GE1YS3FYYM9Z5512MN
:title: Keep one run per root item
:status: accepted
:kind: functional
:derives_from: SCN-CONCURRENT-AGENTS

An item shall have one run rather than independent executions per worktree or caller. Requests to start another run for the same root shall identify the existing run or return an explicit conflict. Other callers may read its progress. Supporting template applications and parallel work on distinct child items belong to that existing execution scope. Finalization metadata and behavior after squash remain to be specified without introducing competing runs.
:::

:::mara requirement REQ-GRAPH-PROGRESS
:mid: 01M3KVG3H37C19FFR0GBK83PV8
:title: Express review progress through ordinary work items
:status: accepted
:kind: functional
:derives_from: SCN-REPEATABLE-REVIEW

Work shall support review and correction progress using ordinary work items and relationships without a structured review-outcome engine. When findings exist, the executor shall create and link required finding/fix, consolidation, and subsequent review work before completing the current review; existing downstream work must remain blocked by that new work. With no findings, completing the review shall introduce no additional blockers. Work shall not parse Markdown findings or classify review/CI results. Provider interpretation and creation of the appropriate graph belong to external executors.
:::

:::mara requirement REQ-GIT-CHECKOUT
:mid: 01M3KVG3HQ6NJ619T146R4GJ91
:title: Require a Git working checkout
:status: accepted
:kind: constraint
:derives_from: SCN-PLAN-WORK

The initial version shall operate in a Git working checkout, including linked worktrees. A non-Git directory or bare repository without a working checkout shall not be treated as a supported project. Shared operational paths shall be resolved through the Git common directory rather than by assuming a checkout-local .git directory.
:::

:::mara requirement REQ-BOOTSTRAP-COMPATIBILITY
:mid: 01M3KY9FH62FYZ71FJVR3GMFXS
:title: Adopt authored bootstrap items without reimport or identity changes
:status: accepted
:kind: constraint
:derives_from: SCN-PLAN-WORK

The first Work implementation shall read and operate on the version-1 durable item files authored before the executable exists. It shall not require a database import, replacement identities, or rewriting bodies merely to adopt the backlog. Format changes shall be explicitly versioned and migrated rather than silently invalidating or reinterpreting those files. Initial acceptance testing shall use an isolated copy of the real bootstrap items.
:::
