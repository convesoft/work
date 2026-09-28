# Work

A local, agent-first issue tracker, being built in Rust. Work will track work items, dependencies, and ownership through a CLI and MCP interface backed by the same operations. External tools execute agents and manage Git worktrees, pull requests, and CI.

## Current state

This is the planning baseline. It contains the product corpus, the version-1 item format, and real implementation issues. There is no Work executable or Cargo project yet, and no installation command is available.

- [Product knowledge](docs/index.mara.md): scope, requirements, design, and decisions.
- [Implementation backlog](.work/README.md): actual Work items, maintained manually until the tool exists.
- [Item format](docs/item-format.mara.md): the contract the implementation must adopt.
- [Delivery and release conventions](docs/delivery.mara.md): branches, evidence, and release preparation.
- [Agent instructions](AGENTS.md): how to work in this repository.

## Development

Start with the selected implementation item and its referenced Mara contracts. The first feature branch establishes the Rust foundation and build commands, CI, and release infrastructure. Workflow files are deliberately absent from this baseline; the generated changelog belongs to a later release-preparation change.

Accepted knowledge describes agreed obligations. It does not establish that a feature exists or that its checks have passed.
