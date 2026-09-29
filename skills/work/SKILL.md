---
name: work
description: Use Work to inspect and manage durable work items, relationships, completion, and readiness through its CLI or MCP tools in a selected Git checkout. Applies whether the project has a defined workflow or models ad hoc work.
---

# Work item tracking

Work stores durable items as Git-tracked `.work/items/<full-id>.md` files. A consumer chooses its own items, bodies, and graph; Work does not require a particular workflow or interpret Markdown headings as structured acceptance fields. It tracks state and graph eligibility but does not execute work.

This skill describes the currently implemented durable item slice. Check the selected `work` executable's `--version` and command `--help`, or MCP `tools/list`, before relying on a command or tool. Use a matching CLI and MCP version if both are available. A skill installation alone does not install the executable.

## Select the view

- Identify the intended Git working checkout. With CLI, run from that checkout or put `--worktree PATH` before the command. With MCP, pass the optional `worktree` on each tool call when the server's checkout is not the intended one. Linked worktrees can show different content for the same ID.
- Prefer available MCP tools for structured use; use `work --json` when MCP is unavailable. CLI success is `{"ok":true,"result":...}`; failure is `{"ok":false,"error":...}`. MCP tool results put the same payload in `structuredContent`, with domain errors under `structuredContent.error` and `isError:true`.
- Read-only inspection does not reserve work. Choose mutations from the user's goal and the project's own rules; do not assume that every ready item should be started or every completed activity should be closed immediately.

## Choose an operation

| Intent | MCP tool | CLI after `work --json [--worktree PATH]` |
| --- | --- | --- |
| See all valid items or inspect one | `item_list`, `item_inspect` | `item list`, `item inspect ID` |
| Check eligibility or diagnose the selected view | `item_ready`, `item_diagnose` | `item ready`, `item diagnose` |
| Create an item with an opaque body | `item_create` | `item create --title TEXT [--body TEXT|-]` |
| Edit header metadata, preserving the body | `item_update` | `item update ID OPTIONS` |
| Add or remove a relation | `relation_add`, `relation_remove` | `relation add|remove KIND SOURCE TARGET` |
| Record completion or reopen a manual item | `item_close`, `item_reopen` | `item close ID [--reason TEXT]`, `item reopen ID` |
| Inspect or replace a diagnosed invalid source | `item_inspect_raw`, `item_repair` | `item inspect FULL_ID --raw`, `item repair FULL_ID --source -` |

Use the returned canonical full ID for authored relations and durable references. An unambiguous lowercase `w-` prefix works for ordinary inspection and mutations; raw inspection and repair require the full ID. `item_create` generates the ID. MCP `item_repair` accepts complete replacement source bytes as `raw_hex`; the CLI reads them from stdin. Consult command help or MCP input schemas for exact optional fields; omit absent MCP fields rather than passing `null`.

`parent` points from child to parent, and `depends_on` from dependent to prerequisite. These affect completion or readiness. `related` is symmetric context, and `discovered_from` records provenance; neither schedules work. A `manual` item has recorded `open` or `done` state. A `children` aggregate has no recorded state and derives effective completion from a nonempty set of resolved children. Closing or reopening one manual item recomputes the graph without changing other manual items' recorded states. A close reason is opaque text; Work does not classify it.

Existing item bodies are opaque bytes. Metadata and relation operations preserve them. There is no ordinary body-update operation in this slice; `item_repair` is for a diagnosed invalid source, not for editing a healthy item. If the user's task calls for direct file editing, follow that project's file convention and inspect diagnostics afterward.

If an operation fails, inspect `error.code` and any diagnostics. Invalid source or graph state can block `item_ready` and structured mutations while `item_list`, `item_inspect`, `item_diagnose`, and raw inspection remain available. Check reported paths and any recovery copy after a conflict or publication error before retrying. Test state-changing examples on disposable copies when the real items must remain intact.

Current tools do not implement claims, templates, temporary runs, handoffs, sessions, workspaces, agent execution, Git worktree management, pull requests, or CI actions. Do not advertise or simulate those as Work operations.
