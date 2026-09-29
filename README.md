# Work

A local, agent-first issue tracker, being built in Rust. Work will track work items, dependencies, and ownership through a CLI and MCP interface backed by the same operations. External tools execute agents and manage Git worktrees, pull requests, and CI.

## Current state

The `0.1.0-alpha.1` candidate supports durable item operations through the CLI and MCP stdio, with equivalent structured results from the shared core. Claims, templates, temporary runs, sessions, workspaces, handoffs, and run finalization are later work. No installation package has been published yet.

- [Product knowledge](docs/index.mara.md): scope, requirements, design, and decisions.
- [Implementation backlog](.work/README.md): actual Work items, maintained with the Work CLI.
- [Item format](docs/item-format.mara.md): the contract the implementation must adopt.
- [Delivery and release conventions](docs/delivery.mara.md): branches, evidence, and release preparation.
- [Agent instructions](AGENTS.md): how to work in this repository.

## Development

Start with the selected implementation item and its referenced Mara contracts. The toolchain is pinned in `rust-toolchain.toml`; Cargo.lock is committed. Run:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
mara --project "$PWD" --format json schema validate
mara --project "$PWD" --format json project validate
cargo run --locked -- discover [PATH]
cargo run --locked -- --help
```

`work discover` prints the selected worktree root and resolved Git common directory as `key=value` lines. Values percent-encode Unix path bytes outside ASCII letters, digits, `/`, `.`, `_`, `-`, and `~`; `%` is always encoded, so paths containing newlines or non-UTF-8 bytes remain reversible and each field stays on one line. `PATH` may point inside another linked worktree; omitting it uses the current directory. A non-Git directory or bare repository returns an unsupported-project error. The shared discovery operation is callable from the library without launching the CLI. The release-preparation item owns the generated changelog, version and license decisions, packaged CLI/MCP smoke checks, and publication evidence.

For the durable item loop, use `work --json item create --title "Task" --body -`, `work --json item list`, `work --json item ready`, and `work --json item inspect ID`. Add a dependency with `work --json relation add depends_on SOURCE TARGET`; close or reopen with `work --json item close ID` and `work --json item reopen ID`. Prefix the command with `--worktree PATH` to select another linked checkout without switching branches. `work --help` lists the implemented commands. [DES-CLI-JSON](docs/design.mara.md) defines the command, JSON, and exit contracts.

[Using the first durable item loop](docs/using-work.md) shows the supported CLI commands and MCP tools, how to start using the existing backlog, and the limits of this slice.
The repository [Work skill](skills/work/SKILL.md) guides an agent through those operations without prescribing a project workflow. It is source guidance; no skill package has been published.

Run `work mcp` from a Git working checkout to serve the same durable operations to an MCP client over stdio. Each tool also accepts an optional `worktree` path. [DES-MCP-STDIO](docs/design.mara.md) defines the first tool and transport contract.

## Alpha distribution

The candidate version is `0.1.0-alpha.1`. The npm dispatcher is `@convesoft/work`; native packages are `@convesoft/work-linux-x64-gnu`, `@convesoft/work-linux-arm64-gnu`, and `@convesoft/work-darwin-arm64`. The supported targets are `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `aarch64-apple-darwin`. After publication, install with `npm install --global @convesoft/work@0.1.0-alpha.1` or run `npx --yes @convesoft/work@0.1.0-alpha.1 --version`. Start the MCP server with `work mcp` from a Git checkout. An unpublished candidate cannot yet be installed from npm.

Work is available under your choice of the [MIT license](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE), recorded as `MIT OR Apache-2.0` in Cargo and npm package metadata. Both full texts are included in native archives and npm packages.

The `release.yml` pipeline captures one main commit, checks source and Mara knowledge, reproduces [the changelog](CHANGELOG.md) with git-cliff 2.13.1, builds native packages on all three hosts, and tests the installed native CLI/MCP binaries. Generate the changelog with `git-cliff --offline --tag v0.1.0-alpha.1 --output CHANGELOG.md`; `cliff.toml` holds the release note template. The protected publication job also tests the packaged dispatcher with the Linux native package before creating a tag or publishing.

Publication requires the GitHub `release` environment and `WORK_RELEASE_READY=true`. Before enabling that variable, verify the environment protection and npm trusted publishing for all four packages against repository `convesoft/work` and workflow `release.yml`, without an npm environment-name restriction. The workflow file alone does not establish those server-side settings. Publication, the annotated tag, and public installation remain separate evidence for [the release item](.work/items/74c22e40cbc249229f86e355a663a942.md).

Accepted knowledge describes agreed obligations. It does not establish that a feature exists or that its checks have passed.
