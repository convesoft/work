# Beta execution handoff — prepared, not dispatched

## Stop point and activation gates

This is a documentation-only preparation. No implementation, PR or review worker has been launched for it, and no claims/runs have been written into live operational storage. Existing items stay open. The deferred storage recovery item w-155dd1c7 is not part of this dispatch.

The canonical source is this committed contract branch. Before implementation activation:

1. Publish and land the contract change using the repository PR process; merging still requires applicable user authorization and exact-head CI/cloud-review evidence. Do not treat this handoff as merge approval. Keep main clean and fast-forward it only after the merge.
2. Run Work readiness against that main. Claims w-299765f4 and runs w-d8c51fea are the intended next parallel pair. Context/session and handoff dependencies now include the capabilities they consume.
3. Verify `HERDR_ENV=1` in the actual launch environment. The final preparation shell reported it unset; do not fake it by exporting a value or control Herdr from an unverified environment. The previously discovered documentation workspace was wG, with pane wG:p1, but rediscover all IDs before any future control. Preserve user focus and unrelated workspaces.
4. Create one Herdr-managed worktree/workspace per item from the then-current main, using branches `feature/w-299765f4-exclusive-claims` and `feature/w-d8c51fea-runs-and-expansion`. Use returned paths and pane IDs; do not guess them. No implementation worktree is created by this preparation.
5. Main supplies the common core interface described below in both worktrees as coordinated edits. Workers may write their owned modules against the agreed contract while main implements shared glue. Main owns compiling the combined surfaces and resolves integration; a worker may not compensate by copying another module or changing its files.

The environment and contract-merge gates are activation prerequisites, not unresolved product decisions. Stop here until the user authorizes resumption; do not silently launch workers after documentation validation.

## Common reading

Read `AGENTS.md`, `.work/README.md`, the selected full item path, all referenced Mara items, `.agents/skills/work-pr-flow/SKILL.md` (main only), and `.codex/agents/work_pr_manager.toml` for later PR handoff. New canonical interfaces:

- `DES-EXECUTION-IO` — `docs/execution-api.mara.md`
- `DES-CLAIM-API` — `docs/claims.mara.md`
- `DES-RUN-API` — `docs/runs.mara.md`
- `DES-CONTEXT-API` — `docs/execution-context.mara.md`
- `DES-FINALIZATION-API` — `docs/run-finalization.mara.md`

IDs/MIDs and accepted status describe knowledge, not implemented features. No separate ownership token, application ledger, material-state overlay, daemon, automatic agent/worktree/Git action or generic recovery engine is permitted. Follow the existing foundation rather than expanding hardening scope. Ask main about a concrete contract contradiction; do not silently design a different product.

## Shared integration ownership — main session

Main owns `src/core/coordination.rs`, `src/core/context.rs`, `src/core/mod.rs`, `src/lib.rs`, `src/core/project.rs`, `src/core/items.rs`, `src/core/graph.rs`, `src/core/operations.rs`, the existing `src/core/storage.rs` and `src/core/storage/*`, all `src/adapters/*`, `src/main.rs`, `Cargo.toml`/`Cargo.lock`, `tests/cli_json.rs`, `tests/mcp_protocol.rs`, and `tests/coordination.rs`/`tests/context.rs`. Main also owns Mara/backlog edits, commits, finding classification and merges.

Common module responsibilities:

- `CoordinationGuard`: one already-held shared/exclusive foundation lock, current metadata, guarded entity IO and source tokens. Mutation services never acquire nested locks.
- `ResolvedView`: actual whole material files selected by persistent item-to-workspace bindings plus run-owned wisps, graph evaluation, source fingerprints and paths. There is no copied execution-state record.
- Basic workspace registration/binding: required for both slices, using `DES-CONTEXT-API` formats immediately. Rich management/named-session commands remain the dependent context item's work.
- Lock-held durable publication primitives and source authorization: load/check graph under shared then sorted checkout locks; preserve current one-file guarantees. Multi-file expansion intentionally exposes partial results on failure.
- Public facades and CLI/MCP mapping: construct trusted core candidates/views, validate claim/session pairs on files being changed, orchestrate completion ordering, expose warnings/partial progress.

Core request/result fields and serialized envelopes are fixed by the Mara API tables. Workers choose private Rust struct/function decomposition inside their owned modules. Main must agree compile-time imports before the first compile; it provides common interfaces, rather than having workers edit or duplicate them. Claims code must accept a validated candidate from main, not a caller-supplied readiness boolean. Run code must accept main's resolved graph/ownership context, not implement a second claim reader. No source stub or implementation is part of this documentation preparation.

Claims can land first with durable candidates only. The runs integration then feeds wisp candidates into the same claim core and repeats exclusion tests. Main carries only necessary shared integration between branches and rebases/reverifies the second PR after the first merges; it does not treat tests on an earlier base as final evidence.

## Worker 1 — exclusive claims

- Canonical item: `.work/items/299765f4605e414f974c2aa7cbe85b49.md`, ID `299765f4605e414f974c2aa7cbe85b49`.
- Proposed unique name: `work_claims_impl` (check live names at launch).
- Own only `src/core/claims.rs`, `src/core/claims/*`, `tests/claims.rs` and `tests/fixtures/claims/*`.
- Deliver acquisition/end format parsers, current/history resolution, authorization, acquire/release/recover/reassign core behavior and owned tests. Main owns material completion/binding/adapters.
- Not this slice: claim-next, session management, run creation, backup/restore or deferred storage repair.
- Required evidence: independent process race yields one owner; same-session reacquisition gets new ID; immutable acquisition bytes and matching terminal record; wrong/stale pair rejection; ended historical context can outlive wisp/session deletion; unavailable foundation and malformed claim/end refusal; release/recover work despite missing item source; explicit reassign partial-gap result; restart persistence. Real CLI/MCP completion and ownership parity are main's integration gates.

Worker instruction: You are not alone in the codebase. Do not revert others' changes or edit unowned files. Accommodate main's shared integration, report needed interface changes, and wait for classification before fixing relayed review findings. Do not commit, push, merge, edit Work/Mara, create live claims/runs, or start agents. Test destructive behavior only in disposable fixtures. Coordinate Cargo ownership with main, use `-j 2`, and report when Cargo is free.

## Worker 2 — runs and mixed expansion

- Canonical item: `.work/items/d8c51fea0fb943a79a3cf057c34fd200.md`, ID `d8c51fea0fb943a79a3cf057c34fd200`.
- Proposed unique name: `work_runs_impl` (check live names at launch).
- Own only `src/core/runs.rs`, `src/core/runs/*`, `src/core/templates.rs`, `tests/runs.rs`, `tests/templates.rs` and `tests/fixtures/runs/*`/`tests/fixtures/templates/*`.
- Deliver run/manifest and wisp parsing, membership and finished calculation, start/attach/detach, template v2/compatible v1 preview, expansion planning and run-side publication through common guarded IO. Main owns workspace resolver, durable writes and adapters.
- Not this slice: implemented squash/discard transitions, named sessions, handoffs or workspace deletion. Understand their reserved formats; prove sequential-run starts from valid terminal fixtures.
- Required evidence: rootless material planning creates no run; mixed template creates files in correct locations and validates relations; preview remains deterministic/read-only; same-root concurrent starts select one current run; material membership conflict; no root auto-completion; empty run not finished; terminal prior run permits a fresh ID; stopped expansion after three of five writes reports/preserves partial files; caller cleanup/retry; source conflicts; restart. Main verifies actual CLI/MCP and resolved worktree integration.

Worker instruction: You are not alone in the codebase. Do not revert others' changes or edit unowned files. Accommodate main's shared integration, report needed interface changes, and wait for classification before fixing relayed review findings. Do not commit, push, merge, edit Work/Mara, create live operational records, or start agents. Use disposable fixtures and coordinate Cargo ownership (`-j 2`) with main.

## Later bounded items

- w-65c88091: exact filters and atomic claim-next in `DES-CLAIM-API`; depends on claims.
- w-6fba939e: session/workspace management in `DES-CONTEXT-API`; depends on claims and runs. Reuse the existing basic bindings.
- w-292b3f92: receiver-scoped handoffs in `DES-CONTEXT-API`; depends on claims and runs.
- w-7a9d4d85: external workspace cleanup begin/report/cancel in `DES-CONTEXT-API`; depends on context and handoffs.
- w-350f6c4f: root digest extensions and explicit wisp discard in `DES-FINALIZATION-API`; depends on runs, handoffs and workspace cleanup.

No additional product questions block these specified interfaces. Private helper naming and test fixture layout are implementation choices. A newly discovered material contradiction still comes to main rather than being patched by inference.

## Future launch and review policy

Only after activation, launch named interactive implementation workers with:

```text
herdr agent start <unique-implementation-name> --kind codex --pane <returned-pane-id> -- -C <actual-worktree-path> -m gpt-6.1-sol -c model_reasoning_effort=high -c service_tier="fast"
```

Create a separate PR-worker pane in each item's returned workspace, preserving focus. Launch explicitly:

```text
herdr agent start <unique-pr-name> --kind codex --pane <returned-pane-id> -- -C <actual-worktree-path> -m gpt-6-luna -c model_reasoning_effort=high -c service_tier="fast"
```

Verify actual model, reasoning, cwd and fast service tier settings before prompting either kind. Starting generic Codex does not select the repository custom agent. PR workers receive the TOML instructions verbatim plus the exact canonical item path/ID, branch, ready-to-push SHA, Mara IDs, test evidence, limitations and existing PR URL. No SHA is ready for implementation publication yet.

Latest user workflow: freeze source edits and run a headless local `codex review --base origin/main` with gpt-6.1-sol, xhigh reasoning and fast service tier in a fresh Herdr review tab before publication. Main classifies findings, verifies accepted fixes and repeats until clean, replacing its previous review tab when the head changes. After publication use automatic cloud review only, including subsequent fixes; no concurrent local/cloud review loops. Never manually request `@codex review`. Required CI and completed automatic review must cover the exact pushed SHA. Report unavailable review accurately; prior PR-specific exceptions are not blanket authorization. Interactive workers use native Codex without --remote or --approve-for-me.

PR workers only publish/monitor/report and perform explicitly directed conversation actions. They never edit source/Work/Mara, commit, force-push or merge. Factual findings may be relayed to implementation workers only after checking their state, with the same URL and reviewed SHA sent to main. Never prompt a blocked agent or blindly resend after a timeout. Main classifies findings, verifies accepted fixes, commits and hands off the new exact SHA. Merge requires applicable explicit user approval. Close an implementation item only after acceptance and merge; preserve clean main and safely remove only created/clean worktrees and Herdr surfaces after approved delivery.

## Verification handoff

For source changes, run meaningful owned tests first, then main runs applicable formatting, build, strict Clippy and all-target tests on the final integrated head. Current CI commands are:

```text
cargo fmt --all -- --check
cargo build --locked -j 2
cargo clippy --locked --all-targets --all-features -j 2 -- -D warnings
cargo test --locked --all-targets -j 2
```

Also run complete Mara schema and project validation, requiring `valid:true`, `evaluation_complete:true`, `has_more:false` and inspect diagnostics. Use real CLI/MCP subprocesses for adapter parity, concurrency and restart behavior. No local test result proves another host or power-loss durability. Stop optional testing once acceptance and required gates are covered; do not restart broad hardening/review loops.

This preparation itself changes documentation/backlog only: its evidence is complete Mara validation, Work diagnostics/readiness, identity/link preservation and `git diff --check`. It does not claim Rust behavior tests passed for the new contracts.
