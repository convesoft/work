# Product intent and vocabulary

The product keeps the user-facing model small: work items, relationships, templates, and runs. Persistence and executor choice are properties of work rather than separate task taxonomies.

:::mara goal GOAL-LOCAL-WORK
:mid: 01M3KANQV8WSP6MN5Y3RQEEGAZ
:title: Plan and track agent-driven development locally
:status: accepted

People and software agents can store planned work, inspect its context, and track its outcome locally across sessions. The tracker provides shared project work state without requiring reconstruction from chat history.
:::

:::mara goal GOAL-GRAPH-PROCESS
:mid: 01M3KANQVFEPAH6QA7YE3Q1DKG
:title: Express repeatable work through an actionable graph
:status: accepted

Agents can discover what can be worked on next from dependencies and reusable templates. Review and pull-request gates fit the same work-item model, reducing repeated procedural instructions.
:::

:::mara goal GOAL-QUIET-HISTORY
:mid: 01M3KANQVQ90MHEZJVNNFEJ76G
:title: Keep useful project history without operational noise
:status: accepted

Durable intent and meaningful results remain inspectable as YAML and Markdown. Temporary execution detail can be kept outside that history and discarded when it is no longer useful.
:::

:::mara actor ACT-OPERATOR
:mid: 01M3KANQVYY0R1WDK5H1B8RGKW
:title: Human operator
:status: accepted

The person who plans work, supplies context and acceptance criteria, inspects progress, and makes product or approval decisions. This is a role rather than an assignee record.
:::

:::mara actor ACT-AGENT
:mid: 01M3KANQW6DJNWZVWTCK6BFY0Q
:title: Software development agent
:status: accepted

An external software agent that reads work, performs the requested activity, records findings or outcomes, and discovers subsequent work. Work is intended to accommodate independent agent sessions rather than one particular vendor.
:::

:::mara term TERM-WORK-ITEM
:mid: 01M3KANQWD090XE4C26708YN1Q
:title: Work item
:status: accepted

A unit of requested activity or an observable completion condition in the work graph. Implementation, review, approval, and a pull-request gate use this shared concept. The term does not imply a particular executor, persistence duration, or command interface.
:::

:::mara term TERM-TEMPLATE
:mid: 01M3KANQWMG896YWW877S52GS6
:title: Template
:status: accepted

A reusable definition of work items and their relationships for a repeatable process. Instantiation creates concrete work items; agents can inspect those items without interpreting a separate procedural prompt.
:::

:::mara term TERM-EPHEMERAL-WORK
:mid: 01M3KANQWTTXG81EDFMTACYR3A
:title: Ephemeral work
:status: accepted

Operational work items stored as inspectable files until explicitly squashed or cleaned, outside version-controlled project history. Ephemeral means limited retention, not in-memory storage or loss on process exit. These items participate in the same work graph as durable items. Their source files, rather than a database-only representation, retain the work content.
:::

:::mara term TERM-DURABLE-STATE
:mid: 01M3KANQX147E716BJ78MYSSX5
:title: Durable project state
:status: accepted

Project intent and results intended to survive loss of local operational state and to be preserved in version-controlled YAML and Markdown. It is distinct from Mara's accepted knowledge status and from an agent currently owning an item.
:::
