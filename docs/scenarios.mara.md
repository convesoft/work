# User and agent scenarios

These flows describe observable outcomes. Draft scenarios explore behavior whose detailed policy is still open.

:::mara scenario SCN-PLAN-WORK
:mid: 01M3KAQ0Q79P8SB8J6FAS69PV9
:title: Plan and resume project work
:status: accepted
:contributes_to: GOAL-LOCAL-WORK
:involves: ACT-OPERATOR
:involves: ACT-AGENT

A human records an item with its description, expected result, and relevant dependencies. A later agent session reads the same durable intent, records progress or a result, and leaves enough project context for another session to continue. Outcome: planned work and its recorded state remain available beyond the originating conversation.
:::

:::mara scenario SCN-NEXT-WORK
:mid: 01M3KAQ0QGHRKFGH2AS1QVH341
:title: Select work from dependency state
:status: accepted
:contributes_to: GOAL-GRAPH-PROCESS
:involves: ACT-AGENT

An agent requests actionable work. The tracker evaluates item state and blocking relationships, exposes eligible work, and explains why other work is blocked. Once a prerequisite is satisfied, its dependent work can become actionable. Outcome: selection follows the work graph rather than a hand-maintained sequence in agent instructions.
:::

:::mara scenario SCN-REPEATABLE-REVIEW
:mid: 01M3KAQ0QR9WKNX3GVQWF7X99P
:title: Instantiate a repeatable review and delivery process
:status: accepted
:contributes_to: GOAL-GRAPH-PROCESS
:involves: ACT-OPERATOR
:involves: ACT-AGENT

An operator or agent applies a template to a development item. The template creates connected work for implementation, review, and applicable pull-request gates. Each activity or condition is represented as a work item. Agents select the eligible activity and report its outcome. Outcome: the process is discoverable through graph state. This scenario does not mandate every example step or authorize automatic merging.
:::

:::mara scenario SCN-QUIET-EXECUTION
:mid: 01M3KAQ0QZFJKB31QCAEGP5BDF
:title: Retain results without retaining every execution step
:status: accepted
:contributes_to: GOAL-QUIET-HISTORY
:involves: ACT-AGENT
:involves: ACT-OPERATOR

Execution creates temporary work items and associated operational detail. When that detail is no longer needed, it can be removed without removing the durable project item or its intentionally retained result. Outcome: durable history preserves useful project information without every claim, heartbeat, or intermediate step.
:::

:::mara scenario SCN-CONCURRENT-AGENTS
:mid: 01M3KAQ0R6XTFTT3Y2MCAF7K5K
:title: Coordinate independent agents across worktrees
:status: accepted
:contributes_to: GOAL-LOCAL-WORK
:involves: ACT-AGENT

Two agents in linked worktrees attempt to claim the same repository work item. Exactly one claim succeeds; both can inspect the current owner and associated workspace. The item has one run, not independent executions in different worktrees or competing runs. Readers may inspect its content and execution state without acquiring execution ownership. Other distinct items can be worked on independently, including parallel children within the same run. After an owner disappears, the controller explicitly releases or reassigns its claim; no timer silently grants ownership to someone else.
:::

:::mara scenario SCN-REBUILD-INDEX
:mid: 01M3KAQ0RDPQANNG3VGK9CKKCN
:title: Recover derived state from durable files
:status: draft
:contributes_to: GOAL-QUIET-HISTORY
:involves: ACT-OPERATOR

With agent execution stopped, an operator rebuilds derived indexes from valid durable and retained ephemeral files. Rebuilding indexes preserves existing coordination records when the database still exists. If the side database was lost, files can reconstruct work content and edges but cannot reconstruct lost claims or observations. Outcome: the tool distinguishes file-derived reconstruction from coordination recovery and does not invent ownership or runtime history.
:::

:::mara scenario SCN-NESTED-DELIVERY
:mid: 01M3KFAJ37RFNJ6TPRP6SBBE8G
:title: Complete an epic through implementation and deployment
:status: accepted
:contributes_to: GOAL-GRAPH-PROCESS
:involves: ACT-AGENT
:involves: ACT-OPERATOR

Epic C uses `completion: children`. Its children are A, a manual delivery item with required backend and frontend children A1 and A2, and B, a manual deployment item that depends on A. A1 and A2 can run concurrently unless explicitly ordered. After both finish, A becomes eligible for integration and final acceptance. Explicit completion of A unlocks B. Completing B makes C effectively done without executing C. Adding or reopening a child makes an aggregate incomplete again.
:::

:::mara scenario SCN-HANDOFF
:mid: 01M3KPWZ1QF6SAF7EVA4X8CD9H
:title: Pass context across continuation and consolidation
:status: accepted
:contributes_to: GOAL-GRAPH-PROCESS
:involves: ACT-AGENT

An agent records context before releasing unfinished work; a later session claims the same item and receives that context. In a transition between activities, five completed fix items supply one handoff to a consolidation item that depends on all five. The handoff remains available after the fixes complete and until its receiving work completes. With multiple receiving items it remains until all finish. External tooling chooses and starts agents; Work exposes context and ownership without orchestrating execution.
:::

:::mara scenario SCN-WORKSPACE-CLEANUP
:mid: 01M3KTFNSZ0TKJPD8AKFP2PCZ3
:title: Finish shared execution with explicit workspace cleanup
:status: accepted
:contributes_to: GOAL-QUIET-HISTORY
:involves: ACT-AGENT
:involves: ACT-OPERATOR

Implementation, reviews, and fixes use one workspace through different sessions and claims. Some sessions are reused by name; one-time workers remain unnamed. After all work requiring that workspace finishes and required results are retained, an external executor performs an explicit cleanup work item. It removes the actual worktree and reports success before Work removes the operational workspace record and associations. If another item or run still needs the shared workspace, cleanup waits. If removal fails, the record remains inspectable and cleanup can be retried.
:::
