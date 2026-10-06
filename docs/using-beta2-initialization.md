# Local beta.2 initialization and material operations

This guide describes the bounded **local source implementation**, not the published beta.1 binary or complete beta.2 acceptance. Build this checkout with `cargo build --locked -j 2`; use the resulting executable. `work --help` and MCP `tools/list` advertise the implemented subset with frozen schemas. The version remains beta.1 during development; a version string is not capability or publication evidence.

## Prepare an existing repository

Use a disposable Git repository for state-changing experiments. Work never runs Git init, creates a sample item, changes branches/worktrees, starts an agent, or registers an external session.

```sh
work --json status
work --json init
work --json status
```

Supported layouts are an ordinary primary `.git` directory, its linked `.git`-file checkouts, and a detached primary checkout. Separate Git directories (`git init --separate-git-dir`) are explicitly unsupported and refuse without Work writes. Root storage always belongs to the actual primary checkout, not the selected linked branch or Git administrative directory.

Init prepares primary `.work/items`, `.work/templates`, root scope/run directories, checkout/repository coordination, and exactly one active standing virtual-root run. It creates no synthetic root item. Healthy repeat is unchanged and preserves run ID, creation time, run/anchor bytes and authored material/template content. Restoring a missing housekeeping ignore rule changes only necessary configuration, not the execution round.

Only primary `.gitignore` is managed. Existing bytes stay an exact prefix. Init appends missing exact lines under a Work-housekeeping comment, accepting LF/CRLF lines and preserving files without a final newline:

```gitignore
/.work/operations.lock
/.work/**/.operation-*
/.work/**/.storage-*
/.work/**/.stage-*
```

Symlink, nonregular and unreadable ignore targets refuse without replacement/following. Init does not broadly ignore `.work`, untrack files, or edit all linked branches. Carry versionable rules to other branches through external Git actions. **`git clean -X` can delete ignored recovery bytes that you still need.** Inspect before cleanup; an ignore rule is not deletion permission. Same-directory ignore-file stages/displaced bytes may live next to `.gitignore`, outside those nested `.work` rules.

## Read-only status and bounded continuation

Status returns frozen `ExecutionInspection`: `state`, `coordination_available`, nullable `root_run_id`, `pending` and source-linked `diagnostics`. It writes no directories/locks and repairs nothing.

- `uninitialized`: no standing root selection. Material planning can still work.
- `ready`: usable authoritative infrastructure, **not** item readiness, a claim, or a running agent.
- `pending`: inspect the operation-specific progress, including malformed records with unavailable parsed fields.
- `unavailable`: infrastructure/source/authority uncertainty, including terminal root selection without matching rotation or lost persisted-execution coordination.

`work --json status --view physical` inspects only the selected checkout, reports `authoritative:false`, and cannot authorize live coordination. MCP protocol `initialize` only negotiates transport; tool `init` performs repository preparation.

An interrupted init exposes its root `StartProgress` and partial paths. Continue only that operation:

```sh
work --json init --continue FULL_OPERATION_ID
```

The plan freezes run ID/time, original anchor and exact ignore before/after hashes. Continuation verifies completed/next steps without changing targets. User edits, conflicting sources and uncertain artifacts refuse and retain evidence. Missing coordination with persisted execution is not automatically recreated. Preparation can stop before publishing the first plan; inspect reported paths rather than assuming an operation ID is a retry receipt. Uncertain unused stages may require explicit caller inspection/resolution before continuation. No generic recovery, rollback, receipt, blind fresh retry, or automatic repair is provided. After a lost response, inspect actual state; removed progress means there is no permanent continuation receipt.

## Material lifecycle without execution initialization

```sh
work --json item create --title 'Check outcome' --body 'Opaque notes'
work --json item close FULL_ID --reason 'Completed externally'
work --json item reopen FULL_ID
work --json item diagnose
work --json item inspect FULL_ID --raw
work --json item repair FULL_ID --source - < corrected-item.md
```

Reads of absent item catalogs are empty and write nothing. Valid material operations bootstrap only required material directories/locks after candidate checks, never ignores, a root anchor or a run. Routing uses whole authoritative files, including uncommitted bytes and dedicated scope hosts. Global `--worktree PATH` selects a Git checkout, not a stale physical mutation source. `--view physical` is read-only.

Close/reopen apply only to unclaimed manual material items. Aggregate completion is derived; neither operation changes other manual items. Opaque body bytes and source mode are preserved. Ordinary completion cleans its own verified obsolete displaced source; explicit repairs retain malformed/misnamed/shadowed sources for caller inspection. Repair requires a diagnosed source/graph problem and the same immutable full ID. It cannot edit healthy bodies, change authority or silently maintain a new stored-scope tree. Canonical/archive interruption keeps exact-byte guarded item-specific repair continuation. A repair may leave existing graph diagnostics, but introduces no new ones.

Current ownership, uncertain authority, invalid structured graphs and pending transitions enforce their applicable guards. Ownership-aware completion/repair and attached close handoffs are **not implemented** in this slice; valid requests for those branches explicitly refuse rather than claim an ending/message was saved. Prepared acquisition fixtures prove refusal/byte preservation only, not downstream lifecycle success.

## Implemented boundaries

The local source advertises 18 operations: `discover`, `init`, `status`, `scope_list`, `scope_inspect`, `scope_host`, `item_create`, `item_list`, `item_inspect`, `item_inspect_raw`, `item_diagnose`, `item_ready`, `item_update`, `item_close`, `item_reopen`, `item_repair`, `relation_add`, `relation_remove`.

All template-specific APIs, including empty template reads and pre-init material-only preview/expansion, belong to [runs/templates w-45f7bb7d](../.work/items/45f7bb7d2c9a43ea993cb7dc8ac8e89d.md) under the user-approved allocation. The current issue branch supplies those APIs and explicit child start/inherit/inspect/list; see [the runs/templates guide](using-beta2-runs-templates.md). This initialization baseline prepares their directories and authority; it does not simulate their operations or claim issue-branch integration. Rotation preparation is internal/read-only, not public ending/replacement. Sessions/claims, handoffs, relocation/adoption, promotion, terminal lifecycle/disposal and all-writer cleanup/combined installed-package acceptance remain separately owned downstream. Historical beta.1 commands are not aliases.

Run the applicable Rust checks, wire checks, complete Mara validation and Work diagnostics, plus the public fixture suites in `tests/onboarding.mjs`, `tests/material_completion.mjs` and `tests/scope_hosting.mjs`. Tests use actual CLI/MCP processes and disposable repositories. The exhaustive fault tracer is Linux/x86_64-only and requires `cc` and ptrace permission; unavailable coverage is not all-host or power-loss acceptance. See [test migration](../tests/MIGRATION.md) for preserved, reactivated and pending obligations. Delivery closure still requires separately authorized scope integration and verification.
