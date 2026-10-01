:::mara design DES-RUN-API
:mid: 01M3TWR8ZHXMCB7SN05JFVJV06
:title: Define run files, membership and mixed template expansion
:status: accepted
:kind: interface
:satisfies: REQ-SINGLE-RUN
:satisfies: REQ-GRAPH-TEMPLATES
:satisfies: REQ-EPHEMERAL-FILES
:satisfies: REQ-RUN-ATOMICITY

## Run and membership files

A run lives at `runs/<run-id>/run.yaml`, with exact version-1 fields `format_version, store_id, recovery_generation, id, root_item_id, created_at, phase, material_items`; optional `default_workspace_id, output_workspace_id, ended_at, cleanup`. `phase` is `active|squashing|discarding|finalized|disposed`. Cleanup is reserved to the exact shape in [[DES-FINALIZATION-API]], not a free-form engine. `ended_at` exists only for finalized/disposed. `material_items` is a sorted set of material item IDs, excluding the root. It is membership, not copies of item records. Root must be a material item; output workspace defaults to its resolved workspace at start and is captured explicitly when available.

Each wisp is a normal version-1 Work item document at `runs/<run-id>/items/<item-id>.md`, with existing frontmatter/body rules unchanged. The containing run establishes its lifetime and membership; do not add a duplicate run ID to each item header. All actual wisp files of the run are members, even if expansion stopped partway. Run material membership is the ID set in the manifest; adding members creates no parent/dependency edges. A material item can be a member of at most one current run; root membership in a different current run is refused for beta to avoid ambiguous nesting. Another run for the root starts only after the earlier run reaches finalized/disposed. Preserve the terminal manifest as a small run record; no new receipt entity.

`finished` is a computed result, never a stored phase: every member obligation is done by graph semantics and no member claim remains active. Empty membership is not finished. Root completion is independent and excluded from the member calculation. A finished active run remains current, can accept more work, and becomes unfinished when a member reopens. Terminal runs accept no new members, wisps or claims. Prior terminal metadata can be inspected; default list shows current runs. Run start never changes root completion.

## Operations

| CLI | MCP | Request | Result |
| --- | --- | --- | --- |
| `run start ROOT [--workspace ID] [--output-workspace ID]` | `run_start` | `root, default_workspace_id?, output_workspace_id?` | `{run,finished,changed}` |
| `run inspect RUN_ID` | `run_inspect` | `run_id` | `{run,finished,members,diagnostics}` |
| `run list [--all]` | `run_list` | `include_terminal` default false | `{runs:[...]}` |
| `run attach RUN_ID ITEM...` | `run_attach` | `run_id, items:[id]` nonempty | `{run,finished,changed}` |
| `run detach RUN_ID ITEM...` | `run_detach` | same | `{run,finished,changed}` |
| `template expand NAME [--run RUN_ID] [--root ITEM] ...` | `template_expand` | preview inputs plus optional `run_id` and authorization | `{run_id, items:[{key,id,persistence,path}], updated:[{id,path}], changed}` |

Start, attach and detach use the shared lock. Start is idempotent for the same root/current run when supplied defaults agree; conflicting defaults return `run_conflict`. Finished-but-unfinalized still returns the current run. Attach/detach accepts material IDs only, validates current membership, and refuses active claims on changed members. Repeated attachment/removal is unchanged. Wisp membership ends only by explicit discard/finalization. Manifest is published last during start, after creating required empty subdirectories; an interrupted directory with no manifest is diagnosed and cannot masquerade as a current run. Return its path for explicit inspection/removal before retry; no run-start recovery journal.

Run inspection reads actual member files and their source paths, full bodies via existing item inspect. The coordinator supplies the resolved graph and claims; the run worker computes membership/finished and owns run IO without reimplementing authorization or locks. No template expansion starts a run implicitly.

## Template format version 2

Version 2 retains version-1 structure and strict single-pass declared text substitution. Add optional item `persistence: material|wisp`, default `material`; it is structural and never interpolated. There is no template-level phase/category. Version 1 continues to parse unchanged and has material items by default. Preview supports both versions, emits persistence on every item, and remains read-only with symbolic local keys.

Root input becomes optional when no edge uses `root`; existing bindings/parameters retain exact-match rules. An expansion with `run_id` derives root from that run; an explicitly supplied different root is invalid. Thus planning can create initial material items without any existing root or run. A template containing wisps requires an explicit current target run at creation, not at read-only preview. Preview with no run validates the symbolic prospective graph; with a run it includes its existing members/wisps and resolved material sources. Extend the current preview request's root to optional rather than inventing a second preview operation.

Material items are created in the caller-selected checkout. Ensure/register that workspace and bind each generated material ID there, so later queries can find it even from another checkout. This does not follow an existing item's binding for the destination of a new ID. Wisps go into the target run. Created material items join that run's `material_items` only when a run was supplied; material-only planning without a run creates no run record.

## Bounded publication and partial results

Under the shared lock, resolve inputs, render, allocate fresh IDs for all local keys, construct the complete prospective graph and authorize every existing source file the edges would modify. Recheck loaded source fingerprints under checkout locks. Validate before the first entity write. Apply edges with existing canonical relation direction/storage rules, including parent cardinality and related deduplication; never introduce another edge store. A reference to an existing item does not automatically enroll it.

Publish generated item files in lexical local-key order, with their final headers/body; forward references are permitted within this already validated publication plan. The coordinator's file primitives must not run the old per-item whole-graph validation between these writes. This is deliberate partial creation, not a hidden transaction. Material bindings precede their corresponding file; an interruption can expose a missing-source binding and a diagnostic. Next write affected existing source files in canonical path order, then update run material membership. Each write remains individually guarded. No permanent application record, retry key, rollback or all-files visibility marker. Concurrent readers cooperating with the lock see the result when it is released, including partial results after an error/crash.

Returned partial progress identifies created/updated paths and allocated key/ID mapping when the process can return. After a process crash the caller inspects item/binding/run files; no promise is made to reconstruct the vanished in-memory mapping. Caller cleanup can remove partial files/bindings and repair any changed existing references before retry. Do not automatically delete useful material files. A retry allocates new IDs. Partial malformed/dangling graphs remain diagnosable; no exception makes them claimable. Hidden stage files follow existing IO retention rules and are not items.

## Acceptance and boundaries

Verify material-only rootless planning; mixed material/wisp locations and cross-kind edges; version-1 compatibility; preview no writes; whole-file worktree resolution; one current run under concurrent starts; material membership exclusivity; derived finish/no implicit root close; and partial failure after three of five item writes with inspectable cleanup/retry. Exercise restart, CLI/MCP parity and unchanged main files.

This item establishes run creation, membership and expansion only. Terminal transitions/squash/discard are implemented by the dependent finalization item using the reserved manifest fields; do not fake their success in this slice. Tests may load valid terminal fixtures to prove a later start can allocate a fresh run. Named-session and handoff operations are later slices. Basic worktree registration/binding needed for material routing is coordinator-owned integration for claims/runs; richer context management remains its own item.
:::
