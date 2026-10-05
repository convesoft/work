# Repository onboarding and execution status

This is accepted beta.2 product knowledge, not an implemented command or passing runtime result. The [wire specification](beta2-wire.mara.md) and [scope model](scopes.mara.md) define the shared envelopes and authority model. Existing beta.1 usage remains documented separately until the replacement runtime is delivered.

:::mara requirement REQ-REPOSITORY-ONBOARDING
:mid: 01M46GE66T40FZ30RRCSX9S21R
:title: Prepare a repository and expose read-only execution health
:status: accepted
:kind: functional
:derives_from: SCN-SCOPED-EXECUTION

Work shall provide explicit repository onboarding that prepares the required local files and establishes one active standing virtual-root run without creating a synthetic task or executing Git, agents or external sessions. Repeated healthy onboarding shall preserve that run and existing authored work. Work shall expose initialization, coordination and pending-operation health through a read-only status operation; usable infrastructure shall not imply graph readiness or process activity.

Material CRUD and material-only template planning shall remain available without execution initialization. Truly absent catalogs are empty for reads; valid material writes may prepare only necessary directories and locks, never implicitly establish a run. Corrupt, unreadable or ownership-bearing persisted state shall not be treated as absence. Onboarding may install only exact Work housekeeping ignore rules, preserving authored configuration and leaving authoritative execution records and continuation plans eligible for Git versioning.
:::

:::mara design DES-REPOSITORY-ONBOARDING
:mid: 01M46GE691AJ55HXWWADPTPRSV
:title: Use explicit init, read-only status and housekeeping-only ignores
:status: accepted
:kind: interface
:satisfies: REQ-REPOSITORY-ONBOARDING
:satisfies: REQ-SCOPED-RUNS

## Public operations

Use `work init [--continue ID]` / MCP `init` and `work status [--view resolved|physical]` / MCP `status`. They use the existing ExecutionMutationResult / ExecutionResult envelopes and global checkout selection. The planned `execution init` / `execution inspect` spellings are replaced, not aliases. MCP protocol initialize remains transport negotiation only.

## Preparation and default execution

Require a supported existing Git working repository, including ordinary .git directories, linked checkout .git files, detached primary checkouts and separate Git directories. Resolve the actual primary working checkout, not an administrative directory. The primary hosts root regardless of the caller's linked checkout. Prepare its required .work/items, .work/templates and root scope/run directories and Git-local coordination with the normal publication safeguards. No Git init, branch/worktree operation, sample task, synthetic parent, external executor or session is created.

Validate arguments, graph/candidate, authority and configuration before bootstrap writes. Fresh material writes independently prepare only their needed directories/locks after these checks; reads of absent item/template catalogs report empty/uninitialized state and create nothing. Hosting-only anchors may remain uninitialized. A lock is not a run. Never recreate missing coordination when persisted authoritative execution or progress exists, and never ignore obsolete execution to obtain a fresh start.

Successful init leaves exactly one active selected standing root run. Healthy repeated init does not rotate or rewrite it. If only housekeeping rules need appending, init may change configuration while preserving the exact active run and anchor. Other damaged, conflicting or interrupted execution requires inspection and the permitted same-operation continuation, not implicit repair or reset. Named records, claims, child runs, material state and opaque bodies are untouched.

## Ignore-file publication

Init owns only the primary checkout's root .gitignore. It neither edits every linked checkout nor installs machine-specific paths into versioned configuration. The rules are versionable; callers carry them to other branches through ordinary external Git actions. Work must not assume a linked branch already contains them.

Append only missing exact lines, recognizing LF or CRLF line endings, under a Work-housekeeping comment. Preserve the existing file as an exact byte prefix, adding a separating newline only if needed. Existing rules/comments are not rewritten; already-present exact lines are not duplicated. An absent file is created safely; symlink/nonregular/unreadable files refuse without following or replacing them. The closed set is:

```gitignore
/.work/operations.lock
/.work/**/.operation-*
/.work/**/.storage-*
/.work/**/.stage-*
```

Do not introduce .work/, wildcard lock rules, execution-record ignores, progress ignores or secret/config policies. Other reserved publisher names must be explicitly added to this contract before use. Existing user ignore policies and already-tracked files are not automatically unignored/untracked. Git-local coordination/hosting files already live outside versioned content.

Ignored recovery artifacts remain inspectable and may contain needed bytes; ignoring is not authorization to delete. Warn in usage that git clean -X can delete them. [[DES-FILE-PUBLICATION-LIFECYCLE]] still requires ordinary-success cleanup and preservation of uncertain/repair/continuation evidence.

## Bounded initialization plan

Root initialization uses the root form of StartProgress, not a generic journal. Its required ignore_change is {before:Snapshot,after:Hash} for the fixed primary .gitignore path; no arbitrary path or bytes are supplied by the caller. Freeze the active/new run identity, original anchor and deterministic append result before publication. Steps 0..3 cover run creation/verification, selecting/verifying the root anchor, then publishing/verifying ignore rules; remove progress only after all postconditions and required sync/authority checks pass. Required directory creation accompanies its owning step and is reported in partial evidence if it stops.

A repeat that only restores missing ignore rules uses the existing active run ID/creation time and unchanged anchor. Root run creation and non-root run start never share an implicit fallback. Continuation validates every installed step and only the next expected before/after source; a user edit, changed host, conflicting run or changed ignore bytes is conflict. Preserve available paths/known effects and never repeat a new round merely because a response was lost.

## Status interpretation

Status returns ExecutionInspection: state uninitialized, ready, pending or unavailable; coordination_available; root_run_id or null; pending ProgressViews; and source-linked diagnostics. Ready requires a valid active standing root selection and required authoritative execution/coordination certainty, not merely a nonnull run ID. A terminal selected root without a matching pending rotation is inconsistent/unavailable, not the valid ended-child barrier.

Keep malformed progress visible with null unavailable parsed fields and diagnostics. Inactive copies neither activate execution nor independently make unrelated live infrastructure unavailable; relevant uncertain authority still refuses authorization. Physical mode inspects only the selected checkout and cannot assert live coordination authorization. Status never creates directories/locks, repairs state, resumes progress, monitors processes, interprets PR/CI state or proves per-item readiness.
:::

:::mara verification VER-REPOSITORY-ONBOARDING
:mid: 01M46GE6B9PA0202V7KZXWZP4W
:title: Verify fresh onboarding, ignore preservation and read-only status
:status: accepted
:method: test
:level: acceptance
:verifies: REQ-REPOSITORY-ONBOARDING
:verifies: DES-REPOSITORY-ONBOARDING
:validates: SCN-SCOPED-EXECUTION

Use owned disposable ordinary, linked, detached-primary and separate-Git-directory repositories with no .work directory. Through real CLI/MCP, confirm empty item/template reads and status write nothing, valid material creation bootstraps only planning directories/locks, and execution consumers refuse before init. Run init and verify one active root run without a material root task; race independent callers and repeat with identical results/run identity. Preserve existing item/template bodies and user .gitignore bytes, handle absent/LF/CRLF/no-final-newline files, append only missing exact rules and verify Git ignores the named nested artifacts but not normal Work records or progress. Root-only configuration updates must not silently edit linked branches.

Interrupt initialization at each file/directory publication boundary and compare pending/partial evidence, frozen run identity, anchor and ignore fingerprints. Resume only the matching operation; refuse changed user configuration, symlink/nonregular targets, missing initialized coordination and obsolete execution without resetting state. Restoring only missing ignore rules preserves the active round. Inject cleanup/response failures without advertising safe blind replay.

Status must distinguish uninitialized, usable, pending and unavailable; diagnose a terminal root selected without a valid rotation, malformed progress, uncertain authoritative ownership and coordination loss. Preserve the coordination_unavailable code in readiness consumers. An unreadable inactive scope directory must not hide authoritative siblings. Physical status is local/read-only and cannot authorize execution. These are future checks, not passing implementation evidence; installed-package and combined downstream workflow proof remain VER-BETA2-SURFACES.
:::

:::mara decision ADR-EXPLICIT-ONBOARDING
:mid: 01M46GNP4VMGCKVKBK9EANNDQP
:title: Make onboarding explicit without hiding versionable execution
:status: accepted
:justifies: REQ-REPOSITORY-ONBOARDING
:justifies: DES-REPOSITORY-ONBOARDING

Use one discoverable top-level init command for repository preparation plus its standing default run, and status for read-only infrastructure diagnosis. Do not retain an execution namespace containing only init/inspect or add silent aliases before beta.2 delivery. Recording material plans remains independent of starting execution; lazy planning preparation is a bounded file operation, not implicit root-run creation.

Automatically ignore only Work-owned local locks and publication artifacts through init's exact append-only rules. Authoritative versionable execution and narrow continuation plans remain eligible for Git history. Existing user configuration stays intact, and ignored recovery bytes still require inspection/explicit disposition. This balances clean Git status with the inspectable, versionable scope-owned model; it does not add background cleanup, generic recovery or agent/process control.
:::
