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

Rebuilding graph/lookup state preserves all authoritative entity files. Detectable missing/corrupt shared state after initialization is a recovery condition: return available file-based inspection/readiness with a structured storage warning and human warning, without claiming it is safe to dispatch; refuse claims while ownership is uncertain. Normal graph validation still applies: malformed or partially published item/run files must not produce an apparently complete ready graph. Propose explicit repair/restore or recreation, never silently reset ownership. Restoring old claims requires stopped executors and invalidation of former tokens; authenticate a backup against local store identity evidence, including a matching retained identity or intact current store metadata. With no such evidence, refuse restore and offer explicit recreation with reported loss. Missing individual files cannot always be distinguished from intentional absence after arbitrary external deletion; the initial guarantee covers cooperating Work operations and detected corruption, not proof against tampering or total undetectable storage erasure. Detailed formats, operation state transitions, initialization/loss markers, and recovery tests must be settled in the owning implementation item.
:::
