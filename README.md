# Work

A local, agent-first issue tracker, being built in Rust. Work will track work items, dependencies, and ownership through a CLI and MCP interface backed by the same operations. External tools execute agents and manage Git worktrees, pull requests, and CI.

## Current state

This branch contains the first buildable Rust foundation and read-only Git checkout discovery. Item operations and MCP are not implemented yet, and no installation package has been published.

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
```

`work discover` prints the selected worktree root and resolved Git common directory. `PATH` may point inside another linked worktree; omitting it uses the current directory. A non-Git directory or bare repository returns an unsupported-project error. The shared discovery operation is callable from the library without launching the CLI. The release-preparation item owns the generated changelog, version and license decisions, packaged CLI/MCP smoke checks, and publication evidence.

The `release.yml` pipeline captures a main commit, checks source and Mara knowledge, and builds native packages on the three selected hosts. Its protected publication job requires `WORK_RELEASE_READY=true` and the GitHub `release` environment. Before the release item enables it, configure npm trusted publishing for all four `@convesoft/work` packages with repository `convesoft/work` and workflow `release.yml`, without an npm environment-name restriction; configure GitHub environment reviewers separately. The workflow file alone does not verify any of these external settings. The placeholder Cargo version `0.0.0`, missing license, and absent packaged CLI/MCP smoke script intentionally stop candidate verification until release preparation supplies them.

Accepted knowledge describes agreed obligations. It does not establish that a feature exists or that its checks have passed.
