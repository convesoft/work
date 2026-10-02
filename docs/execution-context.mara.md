:::mara design DES-CONTEXT-API
:mid: 01M3TWRD8A0N57SE471KQ7MEXA
:title: Define workspace, session, handoff and cleanup interfaces
:status: accepted
:kind: interface
:satisfies: REQ-WORKTREE-VIEWS
:satisfies: REQ-HANDOFF-CONTEXT
:satisfies: REQ-WORKSPACE-CLEANUP
:satisfies: REQ-EXTERNAL-EXECUTION

## Workspace and material location

Workspace file `workspaces/<id>.yaml` has exact fields `format_version, store_id, recovery_generation, id, path, state, created_at`, optional `branch, commit, cleanup`. State is `open|closing`. Path is the canonical absolute worktree path encoded by [[DES-EXECUTION-IO]]; branch/commit are caller-supplied observations, not live Git state or authorization. Register validates a Git working checkout of the same common directory; re-registering the same canonical path returns the existing ID unchanged. Another repository is `invalid_argument`. Work does not create checkouts.

The material binding in [[DES-EXECUTION-IO]] owns the current source workspace. Set it after validating the same full item ID in the target checkout; refuse any active claim on the item or a closing workspace. Run start's optional workspace defaults to the root's bound workspace, otherwise the selected checkout. That default controls execution context for wisps, not a second material state. Material items use their own binding; claim acquisition on an unbound material item captures and binds its resolved selected checkout before publishing the claim. Run material attachment similarly captures a binding if absent. Existing bindings override run defaults and survive run disposal. Register/bind partial progress is returned; no receipt is introduced.

Basic registration and material binding are needed by claims/runs and owned by their coordinator integration. The later context item exposes reusable-management commands and named sessions. The same file format is used throughout; do not implement a temporary parallel representation.

| CLI | MCP | Request | Result |
| --- | --- | --- | --- |
| `workspace register PATH [--branch B] [--commit C]` | `workspace_register` | `path, branch?, commit?` | `{workspace,changed}` |
| `workspace inspect ID` / `workspace list` | `workspace_inspect` / `workspace_list` | `workspace_id` / none | `{workspace,bindings,users}` / `{workspaces}` |
| `workspace bind ITEM WORKSPACE_ID` | `workspace_bind` | `item,workspace_id` | `{binding,changed}` |
| `workspace unbind ITEM` | `workspace_unbind` | `item` | `{item_id,changed}` |

Unbind is explicit and requires no active claim/current-run membership for that item; it does not delete the material file. Afterwards the caller-selected checkout supplies unbound view semantics. Bind/rebind is the explicit way to establish a surviving source after merge. Missing source bytes are never inferred from a branch name. Read workspace users from bindings, current runs and current claims, not a mutable duplicate registry. Inspection returns `bindings` as item/workspace references and `users` as a derived array: binding entries carry `kind: binding`, item `id`, `effective_done` (null when unresolved), and `diagnostics`; current run entries carry `kind: run_default|run_output|run_material`, run `id` and, for material sources, `item_id`; current claims carry `kind: claim`, claim `id` and `item_id`. Current roots count as material users even when done. These observations explain retention and cleanup prerequisites; an empty array is not authorization to delete a checkout, and unresolved bindings never establish safe cleanup.

## Named sessions

`runs/<run-id>/sessions/<session-record-id>.yaml` contains `format_version, store_id, recovery_generation, id, run_id, name, session`, optional `availability`. Name is nonempty case-sensitive text, unique within the run. Availability is exactly `{state: available|unavailable|unknown, observed_at: RFC3339}`; it is supplied observation, not liveness proof. No named session is required for one-time agents.

`session set RUN NAME --namespace N --session-id S [--availability STATE --observed-at TIME]` / `session_set` takes `run_id,name,session,availability?`, creates or rebinds the same named record and returns `{session_record,changed}`. `session list RUN` / `session_list` returns `{sessions}`. `session remove RUN NAME` / `session_remove` returns `{changed}`. Mutation requires an active run phase; rebinding/removing a name does not alter existing claim facts. Claims continue to authorize against the captured external session. A caller may pass optional `session_record_id` (CLI `--session-record ID`) alongside its acquire request for context; the values must match the named record at acquisition. Work never launches, resumes or kills a session.

## Handoff documents and operations

Each `handoffs/<id>.md` has YAML frontmatter `format_version, store_id, recovery_generation, id, from_items, to_items, created_at`, optional `session, workspace_id`, followed by the opaque supplied body. From/to lists are nonempty unique full item IDs; overlap is allowed. Match existing item body framing/byte preservation. These references are retention/context, not graph edges.

`handoff create --from ITEM... --to ITEM... --body TEXT|-` / `handoff_create` takes `from_items,to_items,body,session?,workspace_id?,authorization?` and returns `{handoff,changed:true}`. Resolve all endpoints and authorize claimed source items; receiving items need no ownership because their files are not changed. `handoff inspect ID` / `handoff_inspect` returns `{handoff}`; `handoff list [--to ITEM]` / `handoff_list` returns `{handoffs}`. `handoff receivers ID --to ITEM...` / `handoff_receivers` replaces only the receiver set under the shared lock and returns `{handoff,changed}`, preserving body bytes. There is no handoff cancellation state: cancel an item through existing close-with-reason semantics.

`handoff prune [ID...]` / `handoff_prune` takes optional `ids` (omission examines all) and returns `{deleted,retained}`. Delete only when every receiver resolves done in the current view; missing/unreadable receivers retain context with diagnostics. Close/completion may perform the same best-effort pruning after item and claim publication; a pruning failure reports remaining context and never rolls back completed work or deletes unresolved receivers. Release/reassign/source closure alone never makes pruning eligible.

For close with context, optional `handoffs` is an array of the create input without authorization; the item's authorization applies to its source. Save each handoff first, close the item, then end its claim. If interrupted, return saved IDs and later inspect/reuse them; do not resubmit already-saved context automatically. An ordinary `handoff create` followed by close is equally valid. No workflow receipt or automatic deduplication is promised.

## External workspace cleanup

The later cleanup item uses the existing workspace's optional `cleanup` mapping, exact fields `item_id, controller_workspace_id, started_at`, optional `failure` (opaque text). Begin validates an executable cleanup item, a different surviving controller checkout, no active claims on the target workspace, no other unfinished users or current-run defaults still needing it, and explicit surviving bindings for material progress. The cleanup item's own target reference is exempt; its execution workspace is the controller. Resolved current roots count as users until transferred or their run ends.

`workspace cleanup begin ID --item ITEM --controller-workspace ID` / `workspace_cleanup_begin` sets closing and saves that context; `{workspace,changed}` returns the external target path. Closing refuses new bindings, defaults and claim assignments, but remains inspectable. The OS lock is released before external removal.

`workspace cleanup report ID --removed` / `workspace_cleanup_report` takes `workspace_id,removed:true`, verifies the expected path is absent and no new users exist, then removes the workspace file and returns `{workspace_id,removed:true,changed}`. No material files or bindings are silently deleted; rebind/unbind first. If the process stops after record deletion, an inspect-not-found result plus absent path establishes completed cleanup; no extra receipt is required. Reporting a failure takes `removed:false,failure`, preserves closing/context for retry. `workspace cleanup cancel ID` / `workspace_cleanup_cancel` returns to open only when the original checkout still exists with the same repository identity, clearing cleanup context. These verbs never perform physical worktree/Git deletion or close the cleanup item automatically.

## Acceptance

Test same-workspace reuse across sessions, material binding persistence across claim/run lifetimes, named-session rebind without ownership transfer, full verbatim incoming handoffs, completion interruption ordering, receiver retarget/prune races, cleanup closing refusal and external-removal interruption. Use disposable repos and real CLI/MCP subprocesses. Snapshot older metadata and assert it is not copied into a run-owned state layer.
:::
