# Use the first durable item loop

Work reads version-1 files directly from `.work/items/<full-id>.md` in the selected Git checkout. Existing authored files need no import or ID migration. Their Markdown bodies are opaque: metadata edits, relation changes, close, and reopen preserve the body bytes. Work may normalize the YAML header and retain hidden recovery copies when replacing a file. Keep `.work/items/` under Git and review intended item changes before committing them.

The commands below are examples for this repository's backlog. Consumers can model their own items and dependencies, or use independent items without a workflow. The [Work skill](../skills/work/SKILL.md) gives agents the same basic operation guidance.

Build the current checkout with `cargo build --locked`; run `target/debug/work` in the commands below (or substitute `cargo run --locked --`). The published alpha predates the storage foundation; use this checkout to try the new storage commands. Run `work --help` for the command list, then `work item --help` or `work item COMMAND --help` for detail. `--json` returns the same help inside a structured result.

## Inspect and advance the backlog

From the desired Git working checkout:

```sh
target/debug/work --json item list
target/debug/work --json item ready
target/debug/work --json item inspect w-e1121992
target/debug/work --json item diagnose
```

`list` includes all valid items, with full IDs, recorded and effective completion, blockers, and direct relations. `ready` includes executable open manual items whose prerequisites resolve, ordered by priority then ID; an empty array means none are ready. `inspect` accepts a full ID or an unambiguous `w-` prefix. A short prefix shared by multiple items fails with `ambiguous_id`. `diagnose` reports malformed sources and graph errors. The aggregate `w-933a6d82` has no recorded `state`; its `effective_done` derives from its children. The release item remains blocked until that aggregate resolves.

For a new item and a dependency, use the returned full IDs:

```sh
target/debug/work --json item create --title 'Check deployment' --body 'Acceptance notes'
target/debug/work --json relation add depends_on NEW_FULL_ID PREREQUISITE_FULL_ID
target/debug/work --json item inspect NEW_FULL_ID
target/debug/work --json item close PREREQUISITE_FULL_ID --reason 'Completed externally'
target/debug/work --json item ready
target/debug/work --json item reopen PREREQUISITE_FULL_ID
```

`--body -` reads body bytes from stdin. `item update ID` edits title, completion mode, priority, parent, labels, model, or thinking; it never edits the body. `relation add|remove KIND SOURCE TARGET` supports `parent`, `depends_on`, `related`, and `discovered_from`. For `parent`, SOURCE is the child. `item close` records completion for a manual item; `--reason` is optional opaque text. `item reopen` removes that reason. Neither action automatically changes the recorded state of another manual item. Only close real work after its acceptance and merge requirements are met. For experiments, copy the backlog into a disposable Git checkout; the repository's [bootstrap guide](../.work/README.md) keeps this live backlog manual during delivery.

Every `--json` invocation writes one JSON object: success has `ok: true` and `result`; failure has `ok: false` and `error.code`. Syntax and invalid arguments exit 2, missing or ambiguous IDs exit 3, invalid source or candidate graph exits 4, conflicts and duplicates exit 5, and discovery or I/O errors exit 1. A malformed file can be inspected with `item inspect FULL_ID --raw`; `item repair FULL_ID --source -` accepts a complete replacement source on stdin. Diagnose and inspect before repairing. Rejected structured mutations leave the candidate unpublished. Direct file edits outside Work's lock can still race with a mutation; inspect the reported paths and recovery copies after a conflict.

Use `--worktree PATH` before a command to read or edit another linked checkout's selected file view without switching the caller's branch. Work does not create that worktree. For a bound material item, normal queries/mutations use the entire actual file in its associated worktree, including uncommitted changes. `item list|inspect|ready|diagnose --view checkout` explicitly inspects the physical selected checkout; raw inspect/repair are always physical. Missing bound sources are diagnosed without falling back to stale main content.

## Preview a reusable template

Version-1 and version-2 templates are YAML files under `.work/templates/`. Discover and validate them before rendering a graph:

```sh
target/debug/work --json template list
target/debug/work --json template validate review_cycle
target/debug/work --json template preview review_cycle --root ROOT_FULL_ID --param change='BUG-7'
```

Preview returns rendered items under template-local keys and their edges without writing items, runs, claims, or permanent IDs. The caller supplies a root only when an edge references it, exactly one value for each declared `{{name}}` text parameter, and any declared existing-item bindings with `--existing NAME=FULL_ID`. Unknown, missing, or unused parameters and unresolved tokens fail validation. See [the template contract](template-format.mara.md) for the version-1 fields and references. Version 2 adds `persistence: material|wisp` per item, defaulting to material. Version 1 remains compatible. See [runs and expansion](runs.mara.md) for the extension.

## Runs and template expansion

Initialize shared storage, then publish material-only plans without a run, or explicitly start a run for mixed material/wisp work:

```sh
target/debug/work --json template expand PLAN_NAME
target/debug/work --json run start ROOT_ID
target/debug/work --json template preview MIXED_NAME --run RUN_ID
target/debug/work --json template expand MIXED_NAME --run RUN_ID
target/debug/work --json run inspect RUN_ID
target/debug/work --json run attach RUN_ID MATERIAL_ID
target/debug/work --json run detach RUN_ID MATERIAL_ID
target/debug/work --json run list --all
```

`expand` accepts the same parameters/existing bindings as preview and `--authorize` pairs for any claimed existing source it changes. It returns permanent IDs mapped to local keys. Material files go to the selected checkout and keep a workspace binding; wisps go to the explicit run's `items/` folder. No run is started implicitly. Run start captures the root workspace as its default/output workspace unless explicit registered workspace IDs are supplied.

One current run exists per root. Repeat start returns that run when defaults agree. `finished` is derived from member completion and absence of member claims; the root is separate. Finishing does not finalize the run, delete wisps or close the root. Current wisps use the ordinary item and claim commands. Explicit discard/squash/finalization remains a later slice.

Expansion validates the complete prospective graph before publication, then writes individual files. On failure, `partial` reports known created/updated paths and allocated IDs where available. Inspect files/bindings, remove the partial result and repair references before retrying; a retry allocates fresh IDs. There is no rollback, application record or automatic cleanup.

## Shared file storage

Linked worktrees share operational folders in the resolved Git common directory. Templates and unbound items come from the selected checkout; bound items come from their associated workspace, including uncommitted changes. Reading an uninitialized repository creates no storage:

```sh
target/debug/work --json storage inspect
target/debug/work --json storage init
```

`init` explicitly initializes a fresh store and is a no-op when healthy. `inspect` reports `state`, `coordination_available`, validated identity/generation, diagnostics and pending operation IDs. Claim acquisition requires available storage and validates ownership files under its shared lock. Item list, readiness, diagnosis and inspection include `storage` and `storage_warning`. Human output sends warnings to stderr while keeping available file data; malformed item graphs still block readiness.

Detected missing, corrupt or interrupted storage is never recreated automatically. Stop affected executors before an explicit reset, inspect the current identity and generation, then supply those exact values:

```sh
target/debug/work --json storage recreate \
  --expected-store-id STORE_ID --expected-generation GENERATION \
  --executors-stopped --acknowledge-loss
```

Omit an expected field only if inspection found no validated evidence for it; omission is not a wildcard. If the root or lock is missing, stop all Work clients and also pass `--all-clients-stopped`. Recreation retains surviving prior files as recovery context, creates empty live entity folders and changes the generation. This discards their live coordination meaning. It is not a backup/restore operation.

For a supported interrupted operation, use the reported ID:

```sh
target/debug/work --json storage recover OPERATION_ID
```

Resuming recreation requires the same current stopped-executor/loss affirmations. Recovery never resets an unrelated newer generation. Unsupported metadata versions require a compatible tool. Backup/restore, run finalization and workspace cleanup remain deferred. Handoffs are supported as independent receiver-scoped files. See [the storage contract](storage.mara.md) for the file formats and recovery rules.

On Linux, creation under a restrictive umask can require access to `/proc/self/fd` to set permissions through a held directory descriptor. If unavailable, the operation reports an error; it does not use an unsafe pathname fallback. On macOS, a umask that prevents opening a newly created directory causes a permission error; use a umask that leaves owner access (for example, `077`). Work preserves the interrupted state for explicit recovery.

## Claims and material ownership

Initialize shared storage explicitly before claiming. Acquisitions are immutable; release/completion creates a separate ending file. A fresh acquisition always has a fresh ID, including the same session returning. Session namespace/ID are opaque caller-supplied identity, not a secret token.

```sh
target/debug/work --json claim acquire ITEM --actor worker --session-namespace codex --session-id SESSION
target/debug/work --json claim list --current
target/debug/work --json claim inspect CLAIM_ID
target/debug/work --json item close ITEM --authorize '{"claim_id":"CLAIM_ID","session":{"namespace":"codex","id":"SESSION"}}'
target/debug/work --json claim release CLAIM_ID --session-namespace codex --session-id SESSION
```

Replace uppercase placeholders with actual IDs. Repeat `--authorize JSON` when changing several claimed sources; MCP mutations take the corresponding `authorization` array. Reads never require ownership. Resolved readiness excludes claimed items; inspection includes the owner and source path. Closing saves the item before ending its claim. If ending fails, inspect the reported publication and retry with the still-current pair. Former pairs cannot authorize changes after completion, release, reassignment or storage recreation.

`claim recover CLAIM_ID --actor controller --reason TEXT --executors-stopped` ends abandoned ownership. `claim reassign` additionally takes the new `--session-namespace` and `--session-id`; a failed replacement may leave an explicitly reported unclaimed gap. These operations do not stop executors. There is no automatic expiry, heartbeat or claim-next yet. Material bindings survive release; subsequent reads continue to use that source workspace. Explicit workspace bind/unbind commands manage source routing; physical cleanup remains a later slice.

## Workspaces and named sessions

`workspace register PATH`, `workspace inspect ID`, `workspace list`, `workspace bind ITEM WORKSPACE_ID` and `workspace unbind ITEM` manage references to externally created checkouts. Rebinding selects the whole authoritative item file; it does not copy state. Workspace inspection derives users from bindings, current runs and claims, with diagnostics for unresolved sources. Unbind refuses active claims/current-run membership.

`session set RUN NAME --namespace N --session-id S` creates or rebinds a run-scoped name; `session list RUN` and `session remove RUN NAME` inspect or remove names. Optional `--availability STATE --observed-at RFC3339` records an external observation, not liveness proof. `claim acquire` accepts `--session-record ID` to capture a matching named session in the item's current run. Later rebinding/removal does not transfer captured claim ownership. Session mutations require an active run. Neither session removal nor claim release deletes a workspace. See [the context contract](execution-context.mara.md); physical cleanup and run finalization remain later operations.

## Receiver-scoped handoffs

Initialize shared storage explicitly. Each handoff is an independent `work/handoffs/<id>.md` file under the Git common directory. Frontmatter records sources, receivers, identity/generation and creation time; the body is the exact caller-supplied UTF-8 text. References deliver context but do not add graph edges or change readiness.

```sh
target/debug/work --json handoff create --from SOURCE --to RECEIVER_A RECEIVER_B --body -
target/debug/work --json handoff inspect HANDOFF_ID
target/debug/work --json handoff list --to RECEIVER_A
target/debug/work --json handoff receivers HANDOFF_ID --to RECEIVER_B
target/debug/work --json handoff prune HANDOFF_ID
```

Repeat `--from` and `--to`, or put multiple item references after either flag. Endpoints resolve to unique full IDs and are sorted on write; the same item may be source and receiver. Create authorizes every claimed source with repeated `--authorize JSON`. Receiver replacement changes only that exact set, preserving body bytes; receiving items need no claim authorization. Optional `--session-namespace N --session-id S` and `--workspace ID` record external context. Entity operations require full handoff IDs. MCP provides `handoff_create`, `handoff_inspect`, `handoff_list` (optional `to_item`), `handoff_receivers` and `handoff_prune` (optional `ids`).

Resolved item inspection and acquisition return complete `incoming_handoffs`, sorted by ID. Multiple sources, multiple receivers and cross-run context are preserved. Unreadable/malformed context is exposed as `handoff_warning` with `incoming_handoffs:null`, not an empty successful lookup. Explicit checkout/raw views remain physical item inspection.

For close with outgoing context, repeat `--handoff JSON`, each containing the create input without authorization; MCP `item_close` accepts the corresponding `handoffs` array. The close request's authorization applies to all claimed sources in these inputs. Save each handoff first, save the item close, then publish its completed claim ending. Success returns `saved_handoffs` in the item snapshot. Errors expose `partial`, known `saved_handoffs`, and the last attempted write's `publication`; inspect uncertain paths before retrying. Finish close with the still-current owner pair without resubmitting already-saved notes. Ordinary handoff create followed by close is equally valid. No receipt, rollback or automatic deduplication is provided.

Close performs retention pruning after item/claim publication. Explicit prune examines the specified IDs, or all when omitted, and returns `deleted`, `retained` (including receiver diagnostics) and `changed`. Only all-resolved receiver sets permit deletion. Source closure, release and reassignment never resolve an unfinished receiver. Cancel a receiver with ordinary `item close --reason TEXT`. Missing/unreadable receivers retain context; reopening after deletion cannot resurrect it. Retain durable results before recipients finish. A pruning failure is an error with remaining context and partial publication evidence; completed work is not rolled back. Handoffs are independent of source runs; finished runs retain context needed by outside receivers. Actual run cleanup/finalization remains the successor item's work.

## MCP stdio

Start `target/debug/work mcp` from a Git checkout and configure an MCP client to launch it over stdio. The server negotiates MCP `2025-06-18` and advertises 45 tools: `discover`, `item_create`, `item_list`, `item_inspect`, `item_inspect_raw`, `item_diagnose`, `item_ready`, `item_update`, `item_close`, `item_reopen`, `item_repair`, `relation_add`, `relation_remove`, `template_list`, `template_validate`, `template_preview`, `storage_inspect`, `storage_init`, `storage_recreate`, `storage_recover`, `claim_acquire`, `claim_inspect`, `claim_list`, `claim_release`, `claim_recover`, `claim_reassign`, `run_start`, `run_inspect`, `run_list`, `run_attach`, `run_detach`, `template_expand`, `handoff_create`, `handoff_inspect`, `handoff_list`, `handoff_receivers`, `handoff_prune`, `workspace_register`, `workspace_inspect`, `workspace_list`, `workspace_bind`, `workspace_unbind`, `session_set`, `session_list`, and `session_remove`. The names correspond to the CLI commands above. Every tool accepts optional `worktree`; otherwise it uses the server process checkout. `item_repair` takes replacement bytes encoded as `raw_hex`. `template_preview` takes `name`, optional `root`/`run_id`, and optional `parameters` and `existing` maps. `template_expand` adds optional `authorization`. Tool successes return the result as `structuredContent`; domain failures set `isError: true` and return `structuredContent.error.code`. Unknown MCP methods and tools are JSON-RPC errors. Use `tools/list` for exact argument schemas; optional fields must be omitted rather than sent as `null`.

## Current limits and verification

This slice manages durable item files, ownership-aware graph readiness, read-only template preview, shared file storage, exclusive claims and basic material-workspace binding. Runs and mixed template publication are supported. Handoffs are supported with receiver-scoped retention and explicit partial completion results. Named sessions and reusable workspace management are supported. Run finalization/discard, workspace cleanup, index rebuild, release actions and executor integration remain later work. It does not start agents, manage worktrees, or perform pull-request or CI actions. In particular, the broader Mara verification definitions covering those later capabilities are future checks, not evidence that they work today. The current acceptance tests include `cargo test --locked --test bootstrap_adoption --test templates --test runs --test run_execution --test coordination --test handoffs --test context_execution --test storage --test cli_json --test mcp_protocol`; they use disposable Git repositories before changing state. Other source and operation contracts are covered by the existing integration tests.
