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

Use `--worktree PATH` before a command to read or edit another linked checkout's selected file view without switching the caller's branch. Work does not create that worktree. If two views contain the same ID with different content, each query uses the selected checkout's content.

## Preview a reusable template

Version-1 templates are YAML files under `.work/templates/`. Discover and validate them before rendering a graph:

```sh
target/debug/work --json template list
target/debug/work --json template validate review_cycle
target/debug/work --json template preview review_cycle --root ROOT_FULL_ID --param change='BUG-7'
```

Preview returns rendered items under template-local keys and their edges without writing items, runs, claims, or permanent IDs. The caller supplies an existing root item, exactly one value for each declared `{{name}}` text parameter, and any declared existing-item bindings with `--existing NAME=FULL_ID`. Unknown, missing, or unused parameters and unresolved tokens fail validation. See [the template contract](template-format.mara.md) for the version-1 fields and references. Publication and permanent ID assignment belong to the later file-backed run slice.

## Shared file storage

Linked worktrees share operational folders in the resolved Git common directory. Items and templates still come from the selected checkout, including uncommitted changes. Reading an uninitialized repository creates no storage:

```sh
target/debug/work --json storage inspect
target/debug/work --json storage init
```

`init` explicitly initializes a fresh store and is a no-op when healthy. `inspect` reports `state`, `coordination_available`, validated identity/generation, diagnostics and pending operation IDs. Foundation availability reports usable storage structure; claims and their enforcement remain a later feature. Item list, readiness, diagnosis and inspection include `storage` and `storage_warning`. Human output sends warnings to stderr while keeping available file data; malformed item graphs still block readiness.

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

Resuming recreation requires the same current stopped-executor/loss affirmations. Recovery never resets an unrelated newer generation. Unsupported metadata versions require a compatible tool. Backup/restore, actual claim operations and entity-specific run/handoff/workspace management remain deferred. See [the storage contract](storage.mara.md) for the file formats and recovery rules.

On Linux, creation under a restrictive umask can require access to `/proc/self/fd` to set permissions through a held directory descriptor. If unavailable, the operation reports an error; it does not use an unsafe pathname fallback.

## MCP stdio

Start `target/debug/work mcp` from a Git checkout and configure an MCP client to launch it over stdio. The server negotiates MCP `2025-06-18` and advertises 20 tools: `discover`, `item_create`, `item_list`, `item_inspect`, `item_inspect_raw`, `item_diagnose`, `item_ready`, `item_update`, `item_close`, `item_reopen`, `item_repair`, `relation_add`, `relation_remove`, `template_list`, `template_validate`, `template_preview`, `storage_inspect`, `storage_init`, `storage_recreate`, and `storage_recover`. The names correspond to the CLI commands above. Every tool accepts optional `worktree`; otherwise it uses the server process checkout. `item_repair` takes replacement bytes encoded as `raw_hex`. `template_preview` takes `name`, `root`, and optional `parameters` and `existing` maps. Tool successes return the result as `structuredContent`; domain failures set `isError: true` and return `structuredContent.error.code`. Unknown MCP methods and tools are JSON-RPC errors. Use `tools/list` for exact argument schemas; optional fields must be omitted rather than sent as `null`.

## Current limits and verification

This slice manages durable item files, graph readiness, read-only template preview, and the shared file-storage foundation. It has no claim or ownership coordination, template publication, temporary runs, handoffs, session or workspace management, index rebuild, release action, or executor. It does not start agents, manage worktrees, or perform pull-request or CI actions. In particular, the broader Mara verification definitions covering those later capabilities are future checks, not evidence that they work today. The current acceptance tests include `cargo test --locked --test bootstrap_adoption --test templates --test storage --test cli_json --test mcp_protocol`; they use disposable Git repositories before changing state. Other source and operation contracts are covered by the existing integration tests.
