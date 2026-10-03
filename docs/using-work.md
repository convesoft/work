# Using Work: durable items and development execution

Work reads version-1 files directly from `.work/items/<full-id>.md` in the selected Git checkout. Existing authored files need no import or ID migration. Their Markdown bodies are opaque: metadata edits, relation changes, close, and reopen preserve the body bytes. Work may normalize the YAML header and retain hidden recovery copies when replacing a file. Keep `.work/items/` under Git and review intended item changes before committing them.

The commands below are examples for this repository's backlog. Consumers can model their own items and dependencies, or use independent items without a workflow. The [Work skill](../skills/work/SKILL.md) gives agents the same basic operation guidance.

Build the current checkout with `cargo build --locked`; run `target/debug/work` in the commands below (or substitute `cargo run --locked --`). The published `0.1.0-alpha.1` supports only the durable loop. This checkout still reports that version but also implements the development execution operations below. Inspect help or MCP `tools/list` to distinguish capabilities; do not substitute an older installed alpha or its MCP schemas. Run `work --help` for the command list, then `work item --help` or `work item COMMAND --help` for detail. `--json` returns the same help inside a structured result.

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

## Filter and atomically claim next work

`item list`, `item ready` and `claim next` share these optional filters, combined with AND:

| CLI | MCP | Meaning |
| --- | --- | --- |
| `--root ITEM` | `root` | Root plus transitive children in the selected graph |
| `--run RUN_ID` | `run_id` | Current run members; excludes its separate root |
| `--label TEXT` repeated | `labels_all` | All exact labels; no label inheritance |
| `--priority-max 0..4` | `priority_max` | Maximum priority number, inclusive |
| `--persistence material\|wisp` | `persistence` | Physical material or run-owned wisp source |

```sh
target/debug/work --json item ready --root ROOT_ID --label backend --priority-max 2
target/debug/work --json claim next --root ROOT_ID --label backend --priority-max 2 \
  --actor worker --session-namespace codex --session-id SESSION
```

Use disposable repositories for claiming examples. Listing never reserves. `claim next` reloads the resolved graph and claims under the shared exclusive lock, chooses an eligible unowned item by priority 0–4 then canonical full ID, and acquires it before unlocking. Success returns `{claim, item, changed:true}`; no eligible item returns `{claim:null,item:null,changed:false}`. `claim_next` takes the same filters plus `actor` and `session:{namespace,id}`. Filters do not remove outside prerequisites from graph evaluation. List remains ID-ordered and includes readiness/ownership explanations; ready is priority-ordered.

Root lookup uses normal item reference errors. Missing runs return `not_found`; terminal runs return `run_not_current`. A valid scope with no matches is empty. `--view checkout --run` is rejected: run filtering requires the resolved view and available coordination. Other filters work in the physical checkout view. Corrupt or unavailable ownership is an error for claim-next, not an empty result.

The shared lock precedes the selected material checkout lock, which stays held through claim publication and the response snapshot. Captured source sets and fingerprints are rechecked; detected changes return `conflict` without automatic retry or choosing another candidate. Inspect any reported partial setup/acquisition and publication status before retrying. These locks serialize cooperating Work callers, not raw editors or external Git actions. Notifications are not needed for exclusion.

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

One current run exists per root. Repeat start returns that run when defaults agree. `finished` is derived from member completion and absence of member claims; the root is separate. Finishing does not finalize the run, delete wisps or close the root. Current wisps use the ordinary item and claim commands. Explicit squash and selected/full discard are supported; see the retention section below.

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

Resuming recreation requires the same current stopped-executor/loss affirmations. Recovery never resets an unrelated newer generation. Unsupported metadata versions require a compatible tool. Backup/restore and the deferred foundation-recovery follow-up remain outside this slice. Handoffs are supported as independent receiver-scoped files. See [the storage contract](storage.mara.md) for the file formats and recovery rules.

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

`claim recover CLAIM_ID --actor controller --reason TEXT --executors-stopped` ends abandoned ownership. `claim reassign` additionally takes the new `--session-namespace` and `--session-id`; a failed replacement may leave an explicitly reported unclaimed gap. These operations do not stop executors. There is no automatic expiry or heartbeat. Use `claim next` for atomic scoped selection. Material bindings survive release; subsequent reads continue to use that source workspace. Explicit workspace bind/unbind commands manage source routing; physical removal is always external.

## Workspaces and named sessions

`workspace register PATH`, `workspace inspect ID`, `workspace list`, `workspace bind ITEM WORKSPACE_ID` and `workspace unbind ITEM` manage references to externally created checkouts. Rebinding selects the whole authoritative item file; it does not copy state. Workspace inspection derives users from bindings, current runs, claims and closing cleanup controller references, with diagnostics for unresolved sources. Unbind refuses active claims/current-run membership.

`session set RUN NAME --namespace N --session-id S` creates or rebinds a run-scoped name; `session list RUN` and `session remove RUN NAME` inspect or remove names. Optional `--availability STATE --observed-at RFC3339` records an external observation, not liveness proof. `claim acquire` accepts `--session-record ID` to capture a matching named session in the item's current run. Later rebinding/removal does not transfer captured claim ownership. Session mutations require an active run. Neither session removal nor claim release deletes a workspace. See [the context contract](execution-context.mara.md). Squash/full discard removes named records only after the run's retention checks pass; external sessions remain outside Work's control.

## Explicit workspace cleanup

Use a surviving control checkout and an ordinary executable cleanup item, distinct from the target. Retain required commits and opaque results externally, then explicitly rebind or unbind all material sources away from the target. Current claims, run defaults/outputs/material sources, and another cleanup's controller references block removal. Completed material bindings still require transfer; unresolved sources do not prove safe cleanup.

```sh
target/debug/work --json workspace cleanup begin TARGET_ID --item CLEANUP_ITEM --controller-workspace CONTROLLER_ID
# External tooling now removes the physical worktree, after Work has unlocked.
target/debug/work --json workspace cleanup report TARGET_ID --removed
# If the external attempt failed instead:
target/debug/work --json workspace cleanup report TARGET_ID --failure 'External removal failed'
# Or cancel while the original checkout still exists in the same repository:
target/debug/work --json workspace cleanup cancel TARGET_ID
```

Begin is the caller's retention attestation, not a Git reachability check. It saves `state: closing` and the cleanup item/controller/start context in the existing workspace file. New bindings/defaults/claim assignments are rejected while closing. Repeating begin for the same item/controller preserves context, failure text and start time with `changed:false`; another cleanup cannot replace it. Required shared handoffs and ended claims remain unchanged, including historical workspace references.

MCP tools are `workspace_cleanup_begin` (`workspace_id,item,controller_workspace_id`), `workspace_cleanup_report` (`workspace_id,removed`, with `failure` required only for false), and `workspace_cleanup_cancel` (`workspace_id`). Failure reporting remains available if the target or item/run graph is unavailable. Success requires an absent target path, valid surviving controller and no remaining users; it removes only the workspace record. Report publication errors identify partial/uncertain paths for inspection. Restart before removal preserves closing context; restart after physical removal continues through report. If reporting stopped after record deletion, inspect-not-found plus absent target establishes completion. Work never physically deletes the checkout, deletes material work, or automatically closes the cleanup item. Use only disposable repositories for experiments.

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

Close performs retention pruning after item/claim publication. Explicit prune examines the specified IDs, or all when omitted, and returns `deleted`, `retained` (including receiver diagnostics) and `changed`. Only all-resolved receiver sets permit deletion. Source closure, release and reassignment never resolve an unfinished receiver. Cancel a receiver with ordinary `item close --reason TEXT`. Missing/unreadable receivers retain context; reopening after deletion cannot resurrect it. Retain durable results before recipients finish. A pruning failure is an error with remaining context and partial publication evidence; completed work is not rolled back. Handoffs are independent of source runs; finished runs retain context needed by outside receivers. Explicit run finalization uses the same receiver-scoped retention rules described below.

## Run digest retention and discard

A finished run remains current and retains its wisps until an explicit cleanup. Squash requires nonempty finished member work and no active claims anywhere in the run; the root may stay open.

```sh
target/debug/work --json run squash RUN_ID --summary -
# Or abandon temporary obligations without a summary:
target/debug/work --json run discard RUN_ID --item WISP_ID --item OTHER_WISP_ID
target/debug/work --json run discard RUN_ID --all
```

Squash writes `.work/digests/<root-id>/<run-id>.md` in the captured output workspace. Its frontmatter contains only `format_version`, `root_item_id`, and `run_id`; its body is the exact summary text. It is an extension owned by the root, not a new Work item. Root inspection exposes `digests` while leaving the original body/state unchanged. If output differs from the root source, explicitly bind the root to that captured output first; Work never moves sources or performs Git actions automatically.

Cleanup freezes the exact wisp/session deletion set in the run manifest before any deletion. While squashing, member edits/source rebindings and outside graph or descendant changes affecting their completion are refused, including ordinary item operations, physical repair and template expansion. The root is not automatically frozen. This keeps captured finished obligations stable across partial cleanup without reconstructing deleted wisps; direct editors and external Git changes do not cooperate with this guard. Squash retains the digest first, then deletes wisps and named-session records and marks the run finalized. Full discard omits the digest and marks the run disposed, even for unfinished wisps. Selected discard removes only the chosen wisps, keeps sessions/other work and returns the run to active. Material files, bindings, shared workspace files, historical claims and existing digests remain untouched.

Active target claims and surviving item or needed outside handoff references block deletion, with IDs reported. Internal item references do not block deletion. Eligible completed-receiver handoffs are pruned first. An unresolved handoff whose receivers are all being discarded is retained independently; later pruning reports missing receivers rather than claiming completion. This retention choice is flagged for later product review in `DES-FINALIZATION-API`.

Errors expose known `partial` progress, `remaining_items`, `remaining_sessions` and the last attempted write's publication status. Inspect uncertain paths before retrying. Pending squash requires resupplying the summary; an existing digest must match exactly. Pending discard requires the recorded set and full/subset mode (`--all` means the recorded full set). Terminal matching calls return `changed:false`; a different squash summary conflicts. A completed subset discard has no retry receipt: repeating its missing IDs returns `not_found`. After finalized/disposed, start a new run for a fresh ID. No cross-filesystem transaction or power-loss guarantee is claimed.

MCP adds `run_squash` (`run_id,summary`) and `run_discard` (`run_id` and exactly `all:true` or nonempty `items`). Successful results include `run_id`, `phase`, `deleted`, `changed`, and for squash `digest_path`.

## Connected development workflow (disposable projects only)

Run `node scripts/verify-beta.mjs /absolute/path/to/work` for an executable example, or use `cargo test --locked --test beta_adoption -j 2`. It creates and removes only its own temporary repositories. The packaged smoke runs the identical harness against the installed native binary or npm dispatcher. Git and Node.js >=18 are required.

The workflow intentionally separates planning, execution and retention:

1. Initialize shared storage explicitly. Preview a parameterized version-2 material-only planning template, then expand it without a root/run. Expansion creates material IDs, **not** a run.
2. Create linked checkouts externally and register them. Bind a divergent material source to verify whole-file resolution versus `--view checkout`. Explicitly start a run for the material root, selecting default/output workspaces as needed.
3. Expand a parameterized mixed template into that run. New material items live in the selected checkout; wisps live in the run. Ready children can be independently claimed with `claim next --run RUN_ID`; a competing request can return `storage_busy` while the nonblocking shared lock is held. Inspect/retry that contention; it is not a successful empty selection.
4. Supply external session identity, optionally capture a named session, and save handoffs. Releasing unfinished work preserves incoming context for the next acquisition. A stopped owner needs explicit recover/reassign; a restarted client or later clock does not release its claim. Rebinding a named session does not transfer ownership.
5. Close members with matching `--authorize` pairs, recording single-line close reasons and full opaque results in handoffs or other retained files. Completion saves outgoing context first, then closes the item, then ends ownership. The root's lifecycle stays independent.
6. Before squash, retain results and explicitly remove any retained material relation pointing into the wisp deletion set. Even a **completed** material dependency on a completed wisp blocks cleanup; Work does not silently erase it. Squash with the caller-authored summary only after member obligations are finished. Verify exact root/digest bytes and retained material bindings.
7. For abandonment, use selected discard or full disposal rather than claiming obligations succeeded. A handoff whose unresolved receivers are wholly discarded can remain independently with missing-receiver diagnostics. Do not simulate orphan cleanup.
8. After external merge/retention, explicitly rebind/unbind all material sources away from a workspace. Begin cleanup from a different surviving controller, remove only the disposable worktree externally, then report removal. Cleanup never closes its ordinary item.

The harness also checks every advertised operation, invalid-input no-write behavior, stale authorization, explicit generation recreation/receipt recovery, and concurrent CLI/MCP exclusion without a notification service. It compares complete domain outcomes and resulting entity files after normalizing only fixture identities, paths, clocks and filesystem observations. CLI syntax failures may use `usage` (exit 2), while MCP schema failures use `invalid_argument`; both reject input before mutation. Domain failure codes and payloads are compared directly. This is local behavior evidence, not proof of power-loss durability or every supported host. Existing storage, claims, run and finalization fault-injection tests cover operation-specific interruption boundaries; no generic recovery engine is provided.

## MCP stdio

Start `target/debug/work mcp` from a Git checkout and configure an MCP client to launch it over stdio. The server negotiates MCP `2025-06-18` and advertises 51 tools: `discover`, `item_create`, `item_list`, `item_inspect`, `item_inspect_raw`, `item_diagnose`, `item_ready`, `item_update`, `item_close`, `item_reopen`, `item_repair`, `relation_add`, `relation_remove`, `template_list`, `template_validate`, `template_preview`, `storage_inspect`, `storage_init`, `storage_recreate`, `storage_recover`, `claim_acquire`, `claim_next`, `claim_inspect`, `claim_list`, `claim_release`, `claim_recover`, `claim_reassign`, `run_start`, `run_inspect`, `run_list`, `run_attach`, `run_detach`, `run_squash`, `run_discard`, `template_expand`, `handoff_create`, `handoff_inspect`, `handoff_list`, `handoff_receivers`, `handoff_prune`, `workspace_register`, `workspace_inspect`, `workspace_list`, `workspace_bind`, `workspace_unbind`, `workspace_cleanup_begin`, `workspace_cleanup_report`, `workspace_cleanup_cancel`, `session_set`, `session_list`, and `session_remove`. The names correspond to the CLI commands above. Every tool accepts optional `worktree`; otherwise it uses the server process checkout. `item_repair` takes replacement bytes encoded as `raw_hex`. `template_preview` takes `name`, optional `root`/`run_id`, and optional `parameters` and `existing` maps. `template_expand` adds optional `authorization`. Tool successes return the result as `structuredContent`; domain failures set `isError: true` and return `structuredContent.error.code`. Unknown MCP methods and tools are JSON-RPC errors. Use `tools/list` for exact argument schemas; optional fields must be omitted rather than sent as `null`.

## Current limits and verification

This slice manages durable item files, ownership-aware graph readiness, read-only template preview, shared file storage, exclusive claims, shared scope filters, atomic claim-next and material-workspace binding. Runs and mixed template publication are supported. Handoffs are supported with receiver-scoped retention and explicit partial completion results. Named sessions, reusable workspace management and external workspace cleanup reporting are supported. Run squash and selected/full ephemeral discard are supported with bounded interruption/retry results. Index rebuild, release actions and executor integration remain later work. It does not start agents, manage worktrees, or perform pull-request or CI actions. Accepted Mara verification definitions describe repeatable obligations, not a passing run or permission to simulate unsupported capabilities. The current acceptance tests include `cargo test --locked --test beta_adoption --test bootstrap_adoption --test templates --test runs --test run_execution --test coordination --test handoffs --test context_execution --test workspace_cleanup --test finalization --test storage --test cli_json --test mcp_protocol --test selection`; they use disposable Git repositories before changing state. Other source and operation contracts are covered by the existing integration tests.
