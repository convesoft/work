# Use the first durable item loop

Work reads version-1 files directly from `.work/items/<full-id>.md` in the selected Git checkout. Existing authored files need no import or ID migration. Their Markdown bodies are opaque: metadata edits, relation changes, close, and reopen preserve the body bytes. Work may normalize the YAML header and retain hidden recovery copies when replacing a file. Keep `.work/items/` under Git and review intended item changes before committing them.

The commands below are examples for this repository's backlog. Consumers can model their own items and dependencies, or use independent items without a workflow. The [Work skill](../skills/work/SKILL.md) gives agents the same basic operation guidance.

Build the current checkout with `cargo build --locked`; run `target/debug/work` in the commands below (or substitute `cargo run --locked --`). No package is published yet. Run `work --help` for the command list, then `work item --help` or `work item COMMAND --help` for detail. `--json` returns the same help inside a structured result.

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

## Inspect shared storage

Linked worktrees share SQLite coordination storage under their resolved Git common directory. Item files in each selected checkout remain authoritative; `item list`, `inspect`, and `ready` reload them and reconcile a derived view. If the database is missing after prior use or fails validation, those file-derived queries still return results with `storage_warning` in JSON/MCP and a warning on stderr in human CLI output. Coordination operations require explicit recovery. A `storage_busy` warning means another process holds a SQLite lock; retry after the lock clears. A `storage_unavailable` warning identifies a database access or I/O failure; inspect and repair the underlying access problem before considering recreation.

```sh
target/debug/work --json storage inspect
target/debug/work --json storage rebuild
target/debug/work --json storage backup
```

`storage inspect` does not initialize or change the database. On the first ordinary use, Work initializes an entirely absent store. `storage rebuild` replaces only derived indexes from valid item files and retained run-file digests, preserving coordination records. After stopping active execution, an operator can use `storage restore --backup ENCODED_PATH` with a path returned by backup or migration, or `storage recreate` after a diagnosed fault. A restored older schema reports `requires_migration: true`; run `storage migrate` explicitly before coordination resumes. Recreating reports lost coordination because claims and observations cannot be reconstructed from files.

## MCP stdio

Start `target/debug/work mcp` from a Git checkout and configure an MCP client to launch it over stdio. The server negotiates MCP `2025-06-18` and advertises 19 tools: `discover`, `item_create`, `item_list`, `item_inspect`, `item_inspect_raw`, `item_diagnose`, `item_ready`, `item_update`, `item_close`, `item_reopen`, `item_repair`, `relation_add`, `relation_remove`, `storage_inspect`, `storage_rebuild`, `storage_backup`, `storage_migrate`, `storage_restore`, and `storage_recreate`. The names correspond to the CLI commands above. Every tool accepts optional `worktree`; otherwise it uses the server process checkout. `item_repair` takes replacement bytes encoded as `raw_hex`. Tool successes return the result as `structuredContent`; domain failures set `isError: true` and return `structuredContent.error.code`. Unknown MCP methods and tools are JSON-RPC errors. Use `tools/list` for exact argument schemas; optional fields must be omitted rather than sent as `null`.

## Current limits and verification

This slice manages durable item files, graph readiness, and shared storage diagnostics, backup, migration, and index rebuild. It does not yet claim work or coordinate ownership, parse temporary run graphs, publish templates, manage handoffs or named sessions, release packages, or execute agents. It does not manage worktrees or perform pull-request or CI actions. In particular, broader Mara verification definitions for those later capabilities are future checks, not evidence that they work today. The applicable acceptance tests include `cargo test --locked --test bootstrap_adoption --test storage --test mcp_protocol`; they use disposable Git repositories before changing state. Other source and operation contracts are covered by the existing integration tests.
