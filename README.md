# Work

A local, agent-first issue tracker, being built in Rust. Work will track work items, dependencies, and ownership through a CLI and MCP interface backed by the same operations. External tools execute agents and manage Git worktrees, pull requests, and CI.

## Current state

The current published prerelease is [`0.1.0-beta.1`](https://github.com/convesoft/work/releases/tag/v0.1.0-beta.1), verified at main revision `91a035d3773ed53e61f9757634299a2d7dfeff10`. The earlier alpha supports only the durable item loop.

Beta.1 adds shared file storage, repository-wide claims, run creation/membership, mixed material/wisp template expansion, reusable workspaces, run-scoped named sessions, receiver-scoped handoffs, atomic scoped claim-next, explicit workspace cleanup reporting, caller-authored root digests, and selected/full ephemeral discard. Check command help or MCP `tools/list` for the selected executable's surface. Backup/restore remains outside the published slice. The known shared-store recovery limitation is recorded in [the recovery follow-up](.work/items/155dd1c73ef3412da9d2a44ec57dcfbc.md), now cancelled as superseded by beta.2—not fixed in beta.1. Work never executes agents or creates/deletes physical worktrees. See [the usage guide](docs/using-work.md) for the connected workflow and exact limits.

For development context management, initialize storage explicitly with `work storage init`, then use `work workspace register PATH`, `work workspace bind ITEM WORKSPACE_ID`, and `work workspace inspect WORKSPACE_ID`. `workspace unbind ITEM` removes only the location reference; it refuses current claims or run membership. Explicit rebinding after an external merge/relocation selects the surviving item file, without copying item state.

Use `work session set RUN NAME --namespace PROVIDER --session-id EXTERNAL_ID` to create or rebind a run-scoped name. `session list RUN` and `session remove RUN NAME` inspect or remove names without controlling the external session. Optional `--availability STATE --observed-at RFC3339` records supplied observation only. `claim acquire ITEM --actor ACTOR --session-namespace PROVIDER --session-id EXTERNAL_ID --session-record RECORD_ID` captures matching named context; the name may later be rebound or removed without transferring that claim's ownership. Names and workspaces survive individual item completion and claim release. Named mutations require an active run; squash/full discard removes the run's named records, not external sessions.

For explicit cleanup, retain required commits/results externally and transfer all target material bindings to surviving sources first. From a surviving checkout, use `work workspace cleanup begin TARGET_ID --item CLEANUP_ITEM --controller-workspace CONTROLLER_ID`. Begin records the caller's retention attestation and marks the target closing, refusing new assignments. External tooling then removes the worktree after Work releases its locks. Use `workspace cleanup report TARGET_ID --removed` only after removal, or `--failure TEXT` to preserve retry context. `workspace cleanup cancel TARGET_ID` reopens only the original checkout in the same repository. Work never performs deletion or closes the cleanup item.

The next planned increment is **`0.1.0-beta.2`**, whose accepted [scope model](docs/scopes.mara.md) and [execution contracts](docs/scoped-execution.mara.md) replace global execution bookkeeping with scope-owned, versionable run files. It is **not implemented** by the current binary. The original exact file/API formats are [accepted specification knowledge](docs/scoped-execution.mara.md); the [init/status and hosting-boundary amendment](docs/onboarding.mara.md) needs fresh review/integration before the hosting restart, and the [delivery backlog](.work/README.md#second-beta-scope-owned-execution) tracks implementation and separate release preparation. Delivery now uses [one bounded local beta.2 scope](.work/items/3932a6c6ff404fb0b7cdf0ace756c2a2.md#control-plane-handoff) on `feature/w-3932a6c6-beta2-scope`, with verified issue integration and one eventual scope PR. This design/setup does not bump Cargo/npm versions, regenerate the changelog, or authorize publication.

- [Product knowledge](docs/index.mara.md): scope, requirements, design, and decisions.
- [Implementation backlog](.work/README.md): actual Work items, maintained with the Work CLI.
- [Item format](docs/item-format.mara.md): the contract the implementation must adopt.
- [Delivery and release conventions](docs/delivery.mara.md): branches, evidence, and release preparation.
- [Agent instructions](AGENTS.md): how to work in this repository.

## Development

Start with the selected implementation item and its referenced Mara contracts. The toolchain is pinned in `rust-toolchain.toml`; Cargo.lock is committed. Tests require Git and Node.js >=18 (the connected CLI/MCP harness also runs against installed packages). Run:

```sh
cargo fmt --all -- --check
cargo build --locked -j 2
cargo clippy --locked --all-targets --all-features -j 2 -- -D warnings
cargo test --locked --all-targets -j 2
mara --project "$PWD" --format json schema validate
mara --project "$PWD" --format json project validate
cargo run --locked -- discover [PATH]
cargo run --locked -- --help
```

`work discover` prints the selected worktree root and resolved Git common directory as `key=value` lines. Values percent-encode Unix path bytes outside ASCII letters, digits, `/`, `.`, `_`, `-`, and `~`; `%` is always encoded, so paths containing newlines or non-UTF-8 bytes remain reversible and each field stays on one line. `PATH` may point inside another linked worktree; omitting it uses the current directory. A non-Git directory or bare repository returns an unsupported-project error. The shared discovery operation is callable from the library without launching the CLI. The release-preparation item owns the generated changelog, version and license decisions, packaged CLI/MCP smoke checks, and publication evidence.

For the durable item loop, use `work --json item create --title "Task" --body -`, `work --json item list`, `work --json item ready`, and `work --json item inspect ID`. Add a dependency with `work --json relation add depends_on SOURCE TARGET`; close or reopen with `work --json item close ID` and `work --json item reopen ID`. Prefix the command with `--worktree PATH` to select another linked checkout without switching branches. `work --help` lists the implemented commands. [DES-CLI-JSON](docs/design.mara.md) defines the command, JSON, and exit contracts.

[Using Work](docs/using-work.md) shows the supported CLI commands and MCP tools, how to start using the existing backlog, a disposable connected beta workflow, and the current limits.
The repository [Work skill](skills/work/SKILL.md) guides an agent through those operations without prescribing a project workflow. It is source guidance; no skill package has been published.

Run `work mcp` from a Git working checkout to serve the same item and execution operations to an MCP client over stdio. Each tool also accepts an optional `worktree` path. [DES-MCP-STDIO](docs/design.mara.md) defines the first tool and transport contract.

## Local packaged verification

Build and install the selected candidate artifacts locally without publishing:

```sh
cargo build --locked --release -j 2
scratch=$(mktemp -d)
native=$(node scripts/package-npm.mjs platform x86_64-unknown-linux-gnu target/release/work "$scratch/stage")
native_tgz=$(npm pack "$native" --pack-destination "$scratch" --silent)
dispatcher=$(node scripts/package-npm.mjs main "$scratch/stage")
dispatcher_tgz=$(npm pack "$dispatcher" --pack-destination "$scratch" --silent)
scripts/smoke-packaged.sh "$scratch/$native_tgz" x86_64-unknown-linux-gnu
scripts/smoke-packaged.sh "$scratch/$native_tgz" x86_64-unknown-linux-gnu "$scratch/$dispatcher_tgz"
```

Substitute the available supported target on ARM Linux or Apple Silicon. Keep artifact hashes and the exact source SHA/tree with local evidence: locally packed tarballs are not automatically the verified registry/release artifacts, even when their version matches published beta.1. The published identities are recorded in the release item. For a future candidate, recapture and repeat verification on all supported hosts at its approved final main revision. `scripts/smoke-packaged.sh` installs offline into a disposable prefix, verifies metadata/licenses/executability, and runs the connected `scripts/verify-beta.mjs` acceptance through the installed native binary or dispatcher. The harness exercises all advertised operations through real CLI/MCP processes on equivalent disposable linked repositories, compares semantic results and authoritative files, and checks contention, explicit recovery, digest/discard retention and external cleanup. Local smoke is not other-host, registry-install, release or power-loss evidence.

## Distribution and protected publication

The current published prerelease is [`0.1.0-beta.1`](https://github.com/convesoft/work/releases/tag/v0.1.0-beta.1); the first published version was [`0.1.0-alpha.1`](https://github.com/convesoft/work/releases/tag/v0.1.0-alpha.1). The npm dispatcher is `@convesoft/work`; native packages are `@convesoft/work-linux-x64-gnu`, `@convesoft/work-linux-arm64-gnu`, and `@convesoft/work-darwin-arm64`. The supported targets are `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `aarch64-apple-darwin`. Install with `npm install --global @convesoft/work@0.1.0-beta.1` or run `npx --yes @convesoft/work@0.1.0-beta.1 --version`. Start the MCP server with `work mcp` from a Git checkout.

Work is available under your choice of the [MIT license](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE), recorded as `MIT OR Apache-2.0` in Cargo and npm package metadata. Both full texts are included in native archives and npm packages.

The `release.yml` pipeline captures one main commit, checks source and Mara knowledge, reproduces [the changelog](CHANGELOG.md) with git-cliff 2.13.1, builds native packages on all three hosts, and tests the installed native CLI/MCP binaries. Generate the candidate changelog with `npx --yes git-cliff@2.13.1 --offline --tag v0.1.0-beta.1 --output CHANGELOG.md`; `cliff.toml` holds the release note template. The protected publication job also tests the packaged dispatcher with the Linux native package before creating a tag or publishing.

A `main` push changing `CHANGELOG.md` starts release verification. Publication waits for successful source checks and all supported-host builds, then approval through the protected GitHub `release` environment; no repository-variable readiness switch is required. The `release` PR label is a maintainer convention, not a workflow trigger or authorization check. Before a future publication, verify the environment protection and npm trusted publishing for all four packages against repository `convesoft/work` and workflow `release.yml`, without an npm environment-name restriction. Each trusted publisher must allow direct `npm publish`, which this workflow uses. The workflow file alone does not establish those server-side settings. Required main CI checks, release approval and all four current trusted-publisher configurations must be verified before authorized publication. Use one captured final main revision, an immutable annotated tag at that revision, matching verified artifacts, native packages before the dispatcher, and a public install check before the GitHub release is published. On partial publication, inspect completed stages and retry only that same revision and matching artifact digests. Alpha publication, the annotated tag, and public installation are recorded in [the alpha release item](.work/items/74c22e40cbc249229f86e355a663a942.md). [The beta.1 release item](.work/items/e3640780b04f404bbb3baebfa3a50a38.md) is complete with the final-main annotated tag, successful all-host release/public-install workflow, public package versions and GitHub release evidence. [Beta.2 release preparation](.work/items/3932a6c6ff404fb0b7cdf0ace756c2a2.md) remains separate and open; its future publication requires its own revision, verification and authorization.

Accepted knowledge describes agreed obligations. It does not establish that a feature exists or that its checks have passed.
