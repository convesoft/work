# File storage and entity lifecycles

This is the accepted replacement for the former SQLite plan. It defines architecture and lifecycle boundaries; exact entity envelopes and recovery transitions are completed in their owning implementation items. It does not report implemented behavior.

:::mara design DES-SHARED-FILES
:mid: 01M3RPE9A767RN9EMMN7KNM2AN
:title: Share plain operational files across linked worktrees
:status: accepted
:kind: deployment
:supersedes: DES-SHARED-SQLITE
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-CLAIM-EXCLUSION
:satisfies: REQ-WORKTREE-VIEWS
:satisfies: REQ-INDEX-REBUILD

All authoritative Work state is stored in ordinary Markdown or YAML files. Use the resolved Git common directory's `work/` folder for shared operational entities and temporary execution content. Discover that directory from the selected working checkout; do not assume its `.git` entry is a directory. Linked worktrees share these files; independent clones do not. No SQLite database, custom Git refs, required background daemon, or authoritative aggregate snapshot is part of this architecture.

Keep durable `.work/items/` and `.work/templates/` files in their selected checkout. Shared ownership is repository-wide, but shared execution metadata never overrides a checkout's item body, relationships, or recorded completion. CLI and MCP can select another worktree by path and return its uncommitted content without switching the caller's branch. They can resolve referenced workspace context for a controller without requiring the receiving agent to navigate that workspace. This is an interface boundary, not an operating-system access sandbox.

Shared operational files survive feature-worktree removal and branch switching. Do not make active ownership depend on scanning branch-local claim copies or on whichever checkout currently has the `main` branch. Git's worktree inventory locates checkouts; it does not establish claim ownership. Ordinary Git history retains feature-level intent and results through normal commits and squash merges. Live operational records remain outside versioned history and are not automatically copied to main by merging a branch.

Parse authoritative files to build disposable in-memory graph and lookup structures. The initial file implementation requires no persistent index. A later cache must remain optional and reconstructible and must not acquire ownership authority. Initial deployment is one local repository on a filesystem with working advisory locks and atomic same-filesystem replacement; cross-host and network-filesystem coordination are outside the defined scope.
:::

:::mara design DES-ENTITY-LIFECYCLES
:mid: 01M3RPEDHBGVNF68ZD1BRMT1WD
:title: Give each operational entity its own file and lifetime
:status: accepted
:kind: data
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-EPHEMERAL-FILES
:satisfies: REQ-RUN-FINALIZATION

The layout below separates authoritative entities by responsibility and retention. `shared/` denotes `<resolved Git common directory>/work/`; it is not an additional literal directory. Placeholder IDs denote stable identities, not user-provided path fragments. Item IDs retain their existing format; exact versioned envelopes and other identifier formats must be specified before implementing each entity. References join entities by identity; do not embed copies of all entities inside the run manifest or one repository-wide YAML document.

| Entity | Location | Lifetime and authority |
| --- | --- | --- |
| Durable item or retained digest | `.work/items/<item-id>.md` in the selected checkout | Project intent, relationships, recorded completion, and retained result; ordinary Git history |
| Template definition | `.work/templates/<template>.yaml` in the selected checkout | Reusable versioned graph definition; preview remains read-only |
| Store metadata | `shared/store.yaml` | Repository-local format/identity and recovery generation only; no registry containing all entities |
| Mutation lock | `shared/coordination.lock` | Stable OS lock target; file existence or written PID never grants ownership |
| Claim | `shared/claims/<item-id>.yaml` | One current ownership record per repository item; explicit completion/release/reassignment invalidates its token |
| Workspace | `shared/workspaces/<workspace-id>.yaml` | Externally managed checkout/path and optional revision context, associations and cleanup guard; survives claim release and individual item completion |
| Run | `shared/runs/<run-id>/run.yaml` | Root identity, source/output checkout context, run defaults and publication metadata; does not duplicate item bodies, claims or sessions |
| Temporary item | `shared/runs/<run-id>/items/<item-id>.md` | Same work-item graph model and opaque body; retained until safe explicit finalization |
| Template application | `shared/runs/<run-id>/applications/<application-id>.yaml` | One provenance record per application, with template/input identity and local-key-to-published-ID mapping; retries reuse its publication identity |
| Optional named session | `shared/runs/<run-id>/sessions/<session-record-id>.yaml` | Run-scoped name and opaque external provider/session reference; survives individual claims, removed with successful finalization |
| Handoff | `shared/handoffs/<handoff-id>.md` | Independent receiver-scoped context; may cross run boundaries, so retention follows receivers rather than a source run |
| Operation recovery record | `shared/operations/<operation-id>/operation.yaml` and operation-local staged files | Narrow multi-file publication/finalization intent and progress; retain until recovery and retry obligations are satisfied |
| Retained recovery copy | `shared/recovery/<recovery-id>/` | Explicitly retained prior entity files and provenance; never loaded as live ownership merely because they exist |

A one-time external session needs no separate named-session file. An actor/provider reference and externally supplied availability observation live with the claim or named session they describe; no global agent registry or autonomous polling is introduced. A workspace remains independent of sessions and runs that use it. Run defaults and explicit item workspace overrides must have one authoritative owner in their eventual envelope, rather than disagreeing copies across entity files.

Entity files are inspectable with ordinary tools. Their live mutations go through Work's coordination protocol; editing or deleting them manually while executors run is outside its concurrency guarantee. Run cleanup preserves shared workspace records still in use and cross-run handoffs still needed by receivers. Retain a caller-authored durable digest before deleting temporary items, application provenance, or named-session bindings. A small finalization receipt can outlive run detail for idempotent retries; it is recovery metadata, not a second historical task store. Exact receipt pruning and backup policies require an implementation contract.
:::

:::mara design DES-FILE-COORDINATION
:mid: 01M3RPEGS1WVG1K0CMWY1YK035
:title: Serialize file mutations and recover multi-file operations
:status: accepted
:kind: behavior
:satisfies: REQ-CLAIM-EXCLUSION
:satisfies: REQ-CLAIM-RECOVERY
:satisfies: REQ-RUN-ATOMICITY
:satisfies: REQ-RUN-FINALIZATION
:mitigates: RISK-FILE-DB-CONSISTENCY

Use a short-lived OS advisory exclusive lock on the stable shared `coordination.lock` for cooperating CLI/MCP mutations. The kernel releases it when the owning descriptors close, including process exit. Never delete or replace the lock file as stale-lock recovery: two lock inodes would permit two writers. The lock covers state transitions, not the duration of external execution. A single external coordinator is the normal workflow, but independent processes must still receive correct exclusion.

Within the lock, reload relevant authoritative files, validate the selected graph and current ownership, apply the operation, durably publish its result, then unlock. Claim-next selects and reserves under this same critical section. Reassignment creates a fresh token; every owner-authorized mutation checks that token against the current claim. An external executor is not stopped by invalidating its token. Notifications or an optional socket may ask readers to refresh after a successful write, but missed, delayed, duplicate or absent notifications cannot grant ownership or change the result of a claim.

Single-file replacement uses a validated same-directory stage, file sync, atomic publication and parent-directory sync, with no-overwrite creation when required and explicit source-conflict detection. Keep filesystem accesses bound to validated directories; reject unsafe path traversal, symlink substitution and malformed entity identity. Permission failures, lock contention, invalid formats and interrupted operations need distinguishable diagnostics. Never interpret unreadable or malformed ownership files as an empty set of claims.

A shared lock serializes processes but does not make several files crash-atomic. Template application, completion plus claim release/handoff, workspace cleanup reporting, and digest finalization need narrow operation records and a defined commit/visibility point. Persist intent and staged content before exposing the committed result; readers must not schedule a partial graph. Cooperating graph readers either take a shared lock and check pending operation state or use a validated committed manifest. Restart must reconcile a pending operation before scheduling affected work, preserve uncertainty on failure, and allow an idempotent retry without duplicating items or deleting retained results. Finalization can span shared storage and a different checkout/filesystem; it must not assume a cross-directory rename can commit both.

Combine this lock with existing checkout-local durable-file protection in a single specified order: shared coordination lock, then checkout-local lock(s) in stable canonical-path order. Future operations coupling graph eligibility and ownership must use that order. Raw editors and external Git operations do not obey Work locks; recheck source fingerprints before publication and report detected conflicts, but do not promise a transaction against an uncooperative concurrent writer. The existing durable-only slice remains as implemented until this integration is delivered.

Rebuilding graph/lookup state preserves all authoritative entity files. Detectable missing/corrupt shared state after initialization is a recovery condition: return available file-based inspection/readiness with a structured storage warning and human warning, without claiming it is safe to dispatch; refuse claims while ownership is uncertain. Normal graph validation still applies: malformed or partially published item/run files must not produce an apparently complete ready graph. Report the damaged or missing state and propose explicit recreation; never silently reset ownership. Backup creation and restoration from old backups are deferred from the initial storage foundation. The foundation provides safe file operations, validation, diagnostics and explicit recreation with reported loss; it does not need a backup/restore command or engine. Its testable coordination boundary is an explicit unavailable status for uninitialized, damaged, unreadable or incomplete storage. The later claim implementation consumes that status and refuses claim acquisition when safe exclusion cannot be established. Foundation acceptance does not require a claim command, owner-token validation, or passing claim-behavior tests. When entity-aware backup restoration is later defined, it must require stopped executors and invalidate former tokens. Verify a backup against local store identity evidence, such as a matching retained identity or intact current store metadata; with no such evidence, refuse restore. Missing individual files cannot always be distinguished from intentional absence after arbitrary external deletion; the initial guarantee covers cooperating Work operations and detected corruption, not proof against tampering or total undetectable storage erasure. Detailed formats, operation state transitions, initialization/loss markers, and recovery tests must be settled in the owning implementation item.
:::

:::mara design DES-STORAGE-API
:mid: 01M3RT6GY8BA2E5CVQG49R05ZN
:title: Expose file storage inspection and explicit recovery
:status: accepted
:kind: interface
:satisfies: REQ-SIDE-STATE

The file-storage foundation adds four operations shared by CLI and MCP. This is a contract, not implementation evidence. Existing [[DES-CLI-JSON]] and [[DES-MCP-STDIO]] envelopes and selected-checkout semantics apply.

| CLI command | MCP tool | Inputs beyond optional worktree |
| --- | --- | --- |
| storage inspect | storage_inspect | none |
| storage init | storage_init | none |
| storage recreate | storage_recreate | optional expected_store_id/expected_generation; required executors_stopped:true and acknowledge_loss:true; optional all_clients_stopped:false |
| storage recover OPERATION_ID | storage_recover | required operation_id; optional executors_stopped, acknowledge_loss, all_clients_stopped, each default false |

CLI uses corresponding --expected-store-id ID, --expected-generation ID, --executors-stopped, --acknowledge-loss, --all-clients-stopped flags. IDs are canonical lowercase 32-hex UUIDv4. Required affirmations are checked before discovery/mutation. Recovery of a recreation requires both affirmations again; initialization does not. No backup/restore, stdin, arbitrary-path write, claim, migration or generic operation engine is exposed.

MCP schemas use additionalProperties:false, strings for IDs, booleans for affirmations, and optional worktree string. Nulls, wrong types, duplicate CLI flags, unknown fields and malformed IDs fail before discovery/mutation. Missing expected values mean exact absence of validated evidence, never a wildcard.

Inspection returns {storage:StorageInspection}. Its fields are state (uninitialized|initialized|recovery_required), path, identity_path, lock_path, metadata (null or {format_version,store_id,recovery_generation}), retained_store_id (validated witness ID or null), coordination_available:boolean, storage_warning (primary diagnostic or null), diagnostics:[], pending_operations:[]. A diagnostic has code,message,path (nullable),line (nullable one-based). Pending operations have id (nullable),path,kind (nullable),phase (nullable),supported:boolean. Paths use existing percent encoding. Malformed unknown fields do not get invented values.

Fresh absent root/witness returns uninitialized, false, no warning, and makes no writes. Healthy structure returns initialized,true. Damage, unreadable state, lock contention or pending work returns recovery_required,false with distinct diagnostics when project/path context allows inspection; failures preventing an inspection result use the same error categories.

Mutation success returns {changed,storage,operation_id,recovery_paths,loss}. Healthy init no-op has changed:false, operation_id:null,recovery_paths:[],loss:null. Completed initialization has loss:null. Recreation loss is {coordination_reset:true,missing_or_damaged:boolean,prior_state_path:nullable path}; the response's metadata and retained prior context expose any identity change. Recover returns the same operation identity/generation after validating current state; later conflicting generations refuse replay. Optional result fields are nullable.

Add storage (the inspection object, or null when inspection itself fails) and nullable storage_warning consistently to successful item list/ready/diagnose/inspect/raw results. A hard storage inspection error becomes a primary warning while available durable data remains returned. Human reads print warnings to stderr; JSON retains its sole stdout object. Storage health alone never invalidates an otherwise available file query, initializes storage, grants ownership or makes an invalid selected graph ready. Storage inspect reports health as data with exit 0; failed mutations use the exit table below. Human storage output includes state, availability, validated identity/generation, pending operation IDs and recovery/loss context.

Storage errors have code,message and applicable path,operation_id,recovery_paths,diagnostics,errno. Mutation errors always include publication (not_published|possible|published), including post-publication failures. CLI JSON wraps errors as {ok:false,error:{...}}; MCP uses {error:{...}} with isError:true and matching text. Successful MCP structuredContent equals the CLI result payload.

| Error codes | CLI exit |
| --- | --- |
| invalid_argument | 2 |
| storage_busy, unsafe_path, conflict | 5 |
| invalid_format, unsupported_format, storage_missing, storage_corrupt, recovery_required, identity_mismatch | 4 |
| permission_denied, io | 1 |

Existing command/discovery/graph failure codes retain their meanings. Permission denial and other filesystem I/O remain distinct, with errno where available. Unknown/deferred CLI commands remain usage errors; unknown MCP tools remain protocol errors.

The core API is Storage::new(Project), inspect(), initialize(), recreate(RecreateRequest), recover(operation_id, RecoverRequest). Requests contain the corresponding exact expectations and affirmation booleans. Core structs carry PathBuf and typed state/error/publication enums; adapters alone serialize paths and protocol fields. Inspection returns StorageInspection; mutations return StorageOutcome; all are Result<_,StorageError>. Helpers stay focused and private/crate-private. The adapter contract must be verified through real executable and MCP client subprocesses, including human warnings, before delivery.
:::

:::mara design DES-STORE-FOUNDATION
:mid: 01M3RT72XPHRX6RTVPMJ0TQ4QY
:title: Initialize and recover the shared file store safely
:status: accepted
:kind: behavior
:satisfies: REQ-SIDE-STATE
:satisfies: REQ-WORKTREE-VIEWS
:satisfies: REQ-INDEX-REBUILD

This foundation implements [[DES-FILE-COORDINATION]] without claim/run operations, a database, persistent index, backup/restore, migration engine, or generic transaction engine. It exposes storage structure availability; [[REQ-CLAIM-EXCLUSION]] is enforced by the later claim implementation, including envelope validation and generation-scoped tokens.

### Paths and formats

Let common be the canonical Git common directory discovered from the selected checkout and shared be common/work. Linked worktrees share it; independent clones do not. Keep shared/store.yaml, a stable shared/coordination.lock, and real directories claims/, workspaces/, runs/, handoffs/, operations/, recovery/. A retained common/work.identity.yaml witness outside shared detects deletion of shared after use. It is operational metadata, not tracked Git content. Deletion of all local evidence is undetectable and outside this guarantee.

All foundation YAML is one UTF-8 mapping. Reject duplicate/unknown/missing fields, aliases/anchors/tags, multiple documents, wrong types and unpermitted nulls. All IDs/generations are independent random UUIDv4 values encoded as 32 lowercase hexadecimal characters. Metadata has exactly format_version:1, store_id:string, recovery_generation:string. The witness has exactly format_version:1, store_id:string. Matching intact evidence is required. Recognized unsupported metadata or witness versions refuse mutation and remain preserved; explicit recreation does not downgrade a future format.

Each operations/<id>/operation.yaml is a narrow foundation intent with exactly format_version:1, id, kind (initialize|recreate), store_id, previous_generation (UUIDv4 or null), next_generation, phase (prepared|archived|committed|complete), prior_folders (sorted unique subset of claims,workspaces,runs,handoffs), prior_operations (sorted unique canonical directory IDs), and executors_stopped:boolean. Directory ID equals id. Initialize requires null previous_generation, empty prior lists, false executors_stopped, and never archived. Recreate requires true executors_stopped. Null previous_generation means no validated previous generation, not proven absence of ownership.

Each operation directory retains its exact intended store.yaml and identity.yaml plus narrow prior source/context needed to detect conflicts and resume. Staged bytes and intent must agree. Prior bytes/fingerprints are retained before replacement. IDs and next_generation never change on retry. Any auxiliary retained context has a fixed validated format before it is consumed; arbitrary caller-supplied paths are never replayed. Receipts and recovery files are not live entities and are not automatically pruned. Unknown/malformed pending records yield recovery_required; their bytes are never interpreted as a valid operation. Complete receipts must validate; obsolete complete receipts need not match the current generation for normal inspection, but their explicit replay must check current state.

### Retained source context

Each operation has operations/<id>/context.yaml, a strict version-1 single YAML mapping with exactly format_version:1, store:null|Source, identity:null|Source, folders:{name:Directory}, operations:{id:Directory}. Source has exactly raw_hex (lowercase even-length hex), dev, ino, mode, size (unsigned integers), mtime:[signed seconds,unsigned nanoseconds], ctime:[signed seconds,unsigned nanoseconds]. Directory has exactly dev,ino (unsigned integers). Nanoseconds are less than one billion; source size must equal decoded byte length, and integer values must fit their filesystem representation. These are saved bytes and source preconditions, not executable paths or live entity data.

Folder/operation map keys must exactly equal the intent's prior lists; initialize requires null sources and empty maps. Apply the same strict duplicate/unknown-field, alias/anchor/tag, type and null rejection as the main formats. Sync context before prepared intent. Capture held-directory identities and source bytes/fingerprints before archival or replacement; retries verify the original or archived objects against the retained identities. Preserve unexpected replacement/conflict evidence and refuse continuation rather than overwriting it. Each path is fixed by the validated intent/context schema; no arbitrary path is replayed. This context is limited to foundation metadata and top-level directory identities, not a registry or backup of individual entities.

### Inspection and lock

Inspection creates nothing. If both root and witness are absent, return uninitialized without warning and coordination_available:false. Any existing root remnants, missing required path after use, malformed/unreadable metadata, conflicting evidence or pending intent yields recovery_required with specific diagnostics and coordination_available:false. Healthy matching metadata/witness, required real directories and existing stable regular lock, with no pending/invalid operations and at least one fully validated complete receipt matching the live store identity and generation, yields initialized and true. An empty operation directory or only obsolete complete receipts does not establish the live generation; report storage_corrupt without writing. Valid obsolete complete receipts may coexist with a current matching receipt. Foundation does not parse future claim/run payloads or equate structure availability with valid ownership.

Open existing lock read-only for inspection and take nonblocking shared flock. Mutations take nonblocking exclusive flock. Busy is distinguishable from corruption. Never truncate, unlink, exchange or replace an intact lock. Descriptor closure/process exit releases exclusion; file presence/PID/timestamps do not confer ownership. Use close-on-exec descriptors. Shared lock precedes any checkout-local locks, ordered by canonical path bytes. Existing durable-only operations retain their protocol; later claim operations integrate graph eligibility under the specified order.

Hold validated common/root/directory handles; use descriptor-relative operations with no symlink following, regular-file checks and nonblocking opens for potentially substituted special files. Reject unsafe names, symlink/FIFO/device substitutes and hardlinked mutable files. Recheck pathname-to-device/inode bindings before and after publication. New private directories use explicit 0700 and files 0600 despite umask; do not chmod existing locks during reads. On Linux, making a newly created directory accessible under a restrictive umask may use a pinned O_PATH descriptor through the kernel procfs descriptor path; if that reference is unavailable, fail closed rather than follow a mutable storage pathname or change the process-wide umask. This permission repair applies only to a directory just created by the operation, with identity/binding checks, never to an existing directory. Path output follows existing percent encoding of Unix bytes.

### Initialization

initialize is explicit and healthy-state idempotent. Only absence of both root and witness permits fresh creation. Exclusively create the root and stable lock, sync them, take exclusive flock and recheck all evidence. A concurrent initializer may return busy/recovery_required or a rechecked healthy no-op. Never use init to replace a lost lock or adopt unrelated root remnants.

Generate one operation/store/generation identity. Exclusively create its operation directory; write/sync the intended metadata and witness, then durably publish prepared intent before exposing initialization. Create/sync fixed live directories and recovery/. Publish witness first with no-overwrite same-common-directory staging and parent sync. Publish store metadata last with same-root staging and parent sync. Existing matching published bytes are accepted only for a proven same-operation resume. Conflicting bytes refuse continuation. Durably advance committed then complete, verifying intent against published metadata/witness and path bindings at each transition. Availability remains false until the complete matching operation and structure are validated.

A crash before usable intent leaves detectable remnants and requires explicit recreation, not automatic deletion. Explicit recover for a supported intent resumes only missing steps for that same identity and verifies actual files instead of trusting phase alone. A completed-operation retry succeeds only while current metadata/witness still agree with that intent; a later generation returns conflict rather than regressing state. Committed/complete receipt recovery validates existing required directories and retained archives before any creation or publication; missing or damaged structure refuses replay and requires explicit recreation with a fresh generation. It must never recreate a lost entity directory while retaining the old generation. All terminal live-directory accesses remain read-only after the initial validation as well, so detected disappearance between checks cannot fall back to directory creation.

### Explicit recreation

Recreation requires executors_stopped and acknowledge_loss true on each attempt. A recovery generation fence cannot stop external execution. expected_store_id and expected_generation are exact preconditions rechecked under lock: omitted means no currently validated evidence, not a wildcard. A validated witness can supply identity when metadata is damaged/missing. Conflicting intact identity evidence refuses recreation. Preserve an uncontested valid witness identity; when no valid witness remains, explicitly report identity reset and retain surviving evidence before choosing a new identity. Always choose a fresh generation.

Normally reuse intact root/lock and preserve the lock inode. Missing root or lock additionally requires all_clients_stopped:true, explicitly acknowledging that old descriptors must no longer be in use; only this path may create a replacement lock. An unrelated recreation cannot supersede a pending supported recreation: resume it by ID. Supported means that the operation record, directory ID, intended metadata/witness and retained context all validate together; a valid header alone is insufficient. Explicit recreation may retain malformed or missing-component records opaquely, while unsafe paths and permission/I/O uncertainty still refuse. This includes an intact committed receipt awaiting completion. The terminal-structure loss rule above is the exception: if committed structure is damaged so same-generation replay must refuse, an explicitly acknowledged recreation may retain that receipt and establish a fresh generation; the exception does not authorize superseding prepared or archived operations.

Persist prepared intent, staged next metadata/witness and prior source context before moving surviving live data. Record which fixed live folders and prior operation directories existed. Rename those folders without overwrite into recovery/<operation-id>/prior/ and prior operations into a fixed prior-operation archive, syncing both parents. An interrupted archive step requires exactly one original/archive location; uncertainty or conflict refuses. Unknown prior operation kinds can be retained opaquely by explicit recreation; this does not execute them. Unsafe/noncanonical operation directory names refuse reset. Do not move the current recreation record. Never delete existing recovery copies.

Only after all listed prior folders and operations are retained, durably set archived. Then create new empty fixed live folders. This order prevents a retry from treating newly created empty folders as old state. Preserve prior metadata/witness bytes before replacement. Keep an unchanged valid witness untouched; otherwise publish/sync the replacement witness before publishing new metadata. Then committed and complete as for initialization. Every incomplete phase remains unavailable. Same-ID recovery reuses the chosen generation, requires current affirmations and verifies actual bytes/locations; it never creates another generation or overwrites an unexpected archive.

Return structured loss and retained paths: old operational state is no longer live, and deleted/damaged content may have unknown loss. No claim/token recovery or backup service is implied. Later claims scope authorization to current store identity/generation and claim token.

### Publication and selected views

For each file use a synced same-directory create-new stage; use no-overwrite creation or conflict-checked atomic replacement with retained prior content, then parent sync. Detect observed source-byte/inode changes and retain conflict context. Failures identify not_published, possible or published, affected path, operation ID and recovery paths. An error after name publication is not a successful retry result. A pending record keeps partially published state unavailable.

Selected durable queries reload checkout files and reconstruct their graph on each request; no public rebuild command is needed. This never rewrites shared entity files. Return available file inspection/readiness with storage warnings as defined in [[DES-STORAGE-API]], retaining normal graph errors. Tests use disposable repositories and real subprocesses under [[VER-FILE-COORDINATION]]; process-kill evidence is not proof of power-loss durability. Raw editors and external Git do not obey the protocol; observed conflicts are detected, not a promise of transactions against arbitrary tampering.
:::
