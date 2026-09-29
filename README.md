# Work

A local, agent-first issue tracker, being built in Rust. Work will track work items, dependencies, and ownership through a CLI and MCP interface backed by the same operations. External tools execute agents and manage Git worktrees, pull requests, and CI.

## Current state

This branch supports durable item operations through the CLI and MCP stdio, with equivalent structured results from the shared core. Claims, templates, and temporary runs are still in later delivery items. No installation package has been published.

- [Product knowledge](docs/index.mara.md): scope, requirements, design, and decisions.
- [Implementation backlog](.work/README.md): actual Work items, maintained manually until the tool exists.
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
The repository [Work skill](skills/work/SKILL.md) guides an agent through those operations without prescribing a project workflow. It is source guidance; no skill or executable package has been published yet.

Run `work mcp` from a Git working checkout to serve the same durable operations to an MCP client over stdio. Each tool also accepts an optional `worktree` path. [DES-MCP-STDIO](docs/design.mara.md) defines the first tool and transport contract.

The `release.yml` pipeline captures a main commit, checks source and Mara knowledge, and builds native packages on the three selected hosts. Its protected publication job requires `WORK_RELEASE_READY=true` and the GitHub `release` environment. Before the release item enables it, configure npm trusted publishing for all four `@convesoft/work` packages with repository `convesoft/work` and workflow `release.yml`, without an npm environment-name restriction; configure GitHub environment reviewers separately. The workflow file alone does not verify any of these external settings. The placeholder Cargo version `0.0.0`, missing license, and absent packaged CLI/MCP smoke script intentionally stop candidate verification until release preparation supplies them.

Accepted knowledge describes agreed obligations. It does not establish that a feature exists or that its checks have passed.
