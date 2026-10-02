---
name: work
description: Use Work to inspect and manage durable work items, relationships, completion, and readiness through its CLI or MCP tools in a selected Git checkout. Applies whether the project has a defined workflow or models ad hoc work.
---

# Work item tracking

Work stores durable items as Git-tracked `.work/items/<full-id>.md` files. A consumer chooses its own items, bodies, and graph; Work does not require a particular workflow or interpret Markdown headings as structured acceptance fields. It tracks state and graph eligibility but does not execute work.

This skill describes the durable item, shared storage, claim/run, workspace and named-session slices in this development checkout. The published alpha has fewer operations. Check the selected `work` executable's `--version` and command `--help`, or MCP `tools/list`, before relying on a command or tool. Use a matching CLI and MCP version if both are available. A skill installation alone does not install the executable.

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

## Templates and shared storage

When advertised by the selected binary, template_list/template_validate/template_preview match template list/validate/preview. Preview uses local keys and declared text variables without publishing items, runs or permanent IDs. See the tool schemas and template command help for bindings.

Storage tools storage_inspect/storage_init/storage_recreate/storage_recover match storage inspect/init/recreate/recover. Inspect is read-only; fresh absence needs no warning and creates nothing. Init is explicit and healthy-state idempotent. Linked worktrees share the resolved Git common directory's work/ files; item and template content still comes from the selected checkout.

Item reads expose storage and storage_warning without losing available durable data. coordination_available describes foundation structure only; it does not prove claim ownership. For detected damage, inspect the exact diagnostic and pending operation before choosing explicit recovery. Recreation discards live operational meaning, retains surviving bytes and uses a fresh generation. Require stopped-executor and loss acknowledgements, exact observed identity/generation, and all clients stopped if the root/lock was lost. Never silently recreate or erase the intact lock. Resume only a supported reported operation ID; post-publication errors require inspecting retained context before retry.

## Development execution context

The development binary also advertises claim acquire/inspect/list/release/recover/reassign, run start/inspect/list/attach/detach, and template expand. Claims use a required external `{namespace,id}` session and immutable claim ID; owner mutations carry the matching claim/session pair. Work does not execute that session. Runs hold membership and wisps, not copies of material state. Inspect actual command help and MCP schemas for request details.

Workspace register/inspect/list/bind/unbind match workspace_register/workspace_inspect/workspace_list/workspace_bind/workspace_unbind. Register an existing checkout in the same Git repository; registering its canonical path again reuses its ID. Branch/commit fields are supplied observations, not live Git state. Material binding selects the whole item file; explicit rebind establishes a surviving source after external merge/relocation. Unbind removes only that reference and refuses active claims/current-run context. Workspace users are derived from bindings, current runs and claims; unresolved users do not prove safe cleanup.

Session set/list/remove match session_set/session_list/session_remove. Use `session set RUN NAME --namespace N --session-id S` with optional `--availability STATE --observed-at TIME`. Names are case-sensitive and run-scoped; rebinding retains the record ID. A claim may capture optional named context via `--session-record ID`/`session_record_id`, which must match its supplied external identity and current run at acquisition. Later name removal/rebinding never transfers current ownership or terminates the external session. Individual completion/release does not delete names or workspace context. Named mutations require active run phase; lists also inspect frozen/terminal metadata.

Default item inspection resolves bound material files and live wisps. `--view checkout` (MCP `view:"checkout"`) explicitly selects physical durable files; raw inspect/repair remain physical. Missing bound sources are diagnostics, not permission to fall back to stale data. Test context mutations on disposable repos when the real backlog must remain intact.

Handoff create/inspect/list/receivers/prune expose opaque receiver-scoped context. Item inspection and claiming return incoming handoffs; close can save outgoing context before item completion and claim ending. Pruning waits until all receiver obligations resolve. Inspect command help and MCP schemas for authorization and partial-result details.

Backup/restore, claim-next selection, workspace cleanup, run squash/discard/finalization, agent execution, physical Git worktree management, pull requests and CI actions are not implemented. Do not advertise or simulate those as Work operations.
