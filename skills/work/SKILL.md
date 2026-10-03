---
name: work
description: Use Work to inspect and manage durable work items, relationships, completion, and readiness through its CLI or MCP tools in a selected Git checkout. Applies whether the project has a defined workflow or models ad hoc work.
---

# Work item tracking

Work stores durable items as Git-tracked `.work/items/<full-id>.md` files. A consumer chooses its own items, bodies, and graph; Work does not require a particular workflow or interpret Markdown headings as structured acceptance fields. It tracks state and graph eligibility but does not execute work.

This skill describes the item, storage, claim/run, template, workspace/session, handoff and finalization capabilities in this development checkout. The published alpha has only the durable loop. The unchanged alpha version string alone does not identify these development capabilities. Check the selected `work` executable's `--version` and command `--help`, or MCP `tools/list`, before relying on a command or tool. Use a matching CLI and MCP version if both are available. A skill installation alone does not install the executable.

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

## Scoped selection and claims

When advertised by the selected binary, item_list/item_ready and claim_next share optional root, run_id, labels_all, priority_max (0–4), and persistence (material|wisp). CLI uses --root, --run, repeated --label, --priority-max, and --persistence. Filters combine with AND on the complete selected graph: outside prerequisites still gate work. Root includes transitive children; current run membership excludes the separate root; labels are exact, not inherited. Invalid/missing scope references return errors, not an empty match. A terminal run is run_not_current. --view checkout with --run is invalid_argument; the other filters can inspect physical checkout files.

claim_next matches claim next --actor A --session-namespace N --session-id S plus filters. It selects and reserves under one shared lock, ordered by priority then canonical full ID. No eligible work returns {claim:null,item:null,changed:false}; unavailable or corrupt ownership refuses dispatch. Readiness alone never reserves. Claims are repository-wide across linked worktrees; notifications and process-local state do not determine ownership. claim acquire/inspect/list/release/recover/reassign provide explicit ownership lifecycle. Consult help/tool schemas and the usage guide for session-pair authorization, partial publication and recovery. Do not treat ending a claim as stopping an executor.

## Templates and shared storage

When advertised by the selected binary, template_list/template_validate/template_preview match template list/validate/preview. Preview uses local keys and declared text variables without publishing items, runs or permanent IDs. See the tool schemas and template command help for bindings.

Storage tools storage_inspect/storage_init/storage_recreate/storage_recover match storage inspect/init/recreate/recover. Inspect is read-only; fresh absence needs no warning and creates nothing. Init is explicit and healthy-state idempotent. Linked worktrees share the resolved Git common directory's work/ files; item and template content still comes from the selected checkout.

Item reads expose storage and storage_warning without losing available durable data. coordination_available describes foundation structure only; it does not prove claim ownership. For detected damage, inspect the exact diagnostic and pending operation before choosing explicit recovery. Recreation discards live operational meaning, retains surviving bytes and uses a fresh generation. Require stopped-executor and loss acknowledgements, exact observed identity/generation, and all clients stopped if the root/lock was lost. Never silently recreate or erase the intact lock. Resume only a supported reported operation ID; post-publication errors require inspecting retained context before retry.

## Development execution context

The development binary also advertises claim acquire/next/inspect/list/release/recover/reassign, run start/inspect/list/attach/detach/squash/discard, and template expand. Claims use a required external `{namespace,id}` session and immutable claim ID; owner mutations carry the matching claim/session pair. Work does not execute that session. Runs hold membership and wisps, not copies of material state. Inspect actual command help and MCP schemas for request details.

Workspace register/inspect/list/bind/unbind match workspace_register/workspace_inspect/workspace_list/workspace_bind/workspace_unbind. Register an existing checkout in the same Git repository; registering its canonical path again reuses its ID. Branch/commit fields are supplied observations, not live Git state. Material binding selects the whole item file; explicit rebind establishes a surviving source after external merge/relocation. Unbind removes only that reference and refuses active claims/current-run context. Workspace users are derived from bindings, current runs, claims and closing cleanup controller references; unresolved users do not prove safe cleanup.

Session set/list/remove match session_set/session_list/session_remove. Use `session set RUN NAME --namespace N --session-id S` with optional `--availability STATE --observed-at TIME`. Names are case-sensitive and run-scoped; rebinding retains the record ID. A claim may capture optional named context via `--session-record ID`/`session_record_id`, which must match its supplied external identity and current run at acquisition. Later name removal/rebinding never transfers current ownership or terminates the external session. Individual completion/release does not delete names or workspace context. Named mutations require active run phase; lists also inspect frozen/terminal metadata.

Default item inspection resolves bound material files and live wisps. `--view checkout` (MCP `view:"checkout"`) explicitly selects physical durable files; raw inspect/repair remain physical. Missing bound sources are diagnostics, not permission to fall back to stale data. Test context mutations on disposable repos when the real backlog must remain intact.

Handoff create/inspect/list/receivers/prune expose opaque receiver-scoped context. Item inspection and claiming return incoming handoffs; close can save outgoing context before item completion and claim ending. Pruning waits until all receiver obligations resolve. Inspect command help and MCP schemas for authorization and partial-result details.

Workspace cleanup begin/report/cancel match workspace_cleanup_begin/workspace_cleanup_report/workspace_cleanup_cancel. Retain commits/results externally and explicitly rebind/unbind all target material sources first. `workspace cleanup begin ID --item ITEM --controller-workspace ID` attests retention and marks closing under the shared lock; use a different surviving controller checkout. Work releases the lock before external removal and refuses new assignments while closing. Report external success with `workspace cleanup report ID --removed` (MCP `removed:true`), or failure with `--failure TEXT` (MCP `removed:false,failure`). Retry preserves the original context. Cancel reopens only the original path with the same repository identity. These commands never physically delete worktrees or close the cleanup item. After record deletion, inspect-not-found plus absent target path establishes completed cleanup; no extra receipt is created.

## Retention and finalization

Template expansion never starts a run implicitly. Material-only planning can expand without a run; wisps require an explicit current run. Finished active runs still retain wisps and named sessions, and their material roots remain independent.

`run squash RUN --summary TEXT|-` / `run_squash` (`run_id,summary`) retains the caller's exact summary at `.work/digests/<root-id>/<run-id>.md` in the captured output workspace before deleting run wisps/sessions. It neither edits/closes the root nor creates a digest item. Explicitly bind the root to its intended output source first. Active claims and surviving references block cleanup, including completed material dependencies on wisps: retain results and explicitly remove those relations before squash.

`run discard RUN --item WISP` (repeatable) or `--all` / `run_discard` (`run_id` and exactly `items` or `all:true`) abandons selected or all ephemeral work without a digest. Selected discard keeps the run active and its sessions. Full discard disposes the run; material files/bindings, historical claims, workspaces and existing digests survive. Independent handoffs whose unresolved receivers were all discarded remain inspectable with missing-receiver diagnostics; do not invent orphan cleanup.

Inspect `partial`, `publication` and remaining IDs after an error. Pending cleanup freezes the recorded set; retry squash with the same summary or discard with the same set/mode. Matching terminal repeats are unchanged; completed subset deletion has no retry receipt. Use the [usage guide](../../docs/using-work.md) and actual help/schemas for exact results and safety rules.

## Verification and boundaries

`cargo test --locked --test beta_adoption -j 2` runs the connected public workflow on disposable linked fixtures through both surfaces. `scripts/smoke-packaged.sh` reuses that harness against installed native/dispatcher packages. Tests need Git and Node.js >=18. They inspect real resulting files, compare success/domain errors, and cover claims under contention, explicit recovery, retention and external cleanup. CLI syntax `usage` and MCP schema `invalid_argument` are both rejected input; domain failures retain matching codes. A passing local invocation is not other-host or publication evidence.

Backup/restore, automatic expiry/orphan repair, the deferred storage-recovery follow-up, agent execution, physical Git worktree management, pull requests and CI actions remain outside the implemented slice. Do not advertise or simulate those as Work operations.
