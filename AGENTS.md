# Agent instructions

## Project

This Rust project is a local, agent-first issue tracker for managing work state. Treat its behavior and scope as evolving product knowledge; do not infer detailed features from this short description.

## Source of truth

- Git-tracked `*.mara.md` files are the source of truth for product goals, scenarios, requirements, design, and decisions. `.mara/` defines the project vocabulary and validation rules.
- Before changing behavior, discover and read the relevant Mara knowledge. If it is missing or ambiguous, clarify the product decision and record it in Mara before treating an implementation choice as a requirement.
- Keep implementation and documentation aligned. Update affected Mara knowledge when a product contract changes, and preserve item IDs, MIDs, relationships, and references.
- Use the Mara skill and the project's schema to choose flavours, fields, and relations. Prefer Mara MCP operations; use the matching Mara CLI when MCP is unavailable. Scope operations to this repository's root.
- Validate Mara changes with schema and project validation. Require complete results and `valid: true`; inspect diagnostics rather than assuming a successful command means the knowledge is valid.
- An accepted Mara item records accepted knowledge, not proof of implementation or passing tests. Cite actual code and test evidence when reporting completion.

## Rust work

- Inspect the current `Cargo.toml`, source layout, and existing tests before choosing commands or architecture. This repository may still be at the scaffold stage.
- Make the smallest coherent change that satisfies the relevant Mara contract. Add tests for behavior where they provide useful evidence.
- Once a Cargo workspace exists, run the applicable formatting, build, and test checks for the changed code. Report the checks run and any that could not run.

## Delivery coordination

- Mara owns durable product meaning. `.work/items/` owns delivery tasks, task-specific acceptance criteria, priority, dependencies, and progress. GitHub owns pull requests, reviews, CI results, and merge/publication evidence. Do not introduce Mara task or release flavours to mirror the backlog.
- Read the selected Work item and its Mara references before implementation. If a task or review reveals a product decision, update the canonical knowledge instead of leaving that decision only in a task or PR.
- Follow [the delivery conventions](docs/delivery.mara.md). Start each implementation item on a dedicated branch from current `main`; use `feature/w-<short-id>-<description>` and one bounded PR per item. Squash merging is the default.
- During bootstrap, maintain item files manually according to `.work/README.md`. Keep canonical IDs, filenames, relationships, and unrelated body content intact. Do not invent claims or run records before the coordination layer exists.
- For this repository's implementation tasks, mark done only after their acceptance criteria pass and the change is merged. Release preparation also requires the specified publication evidence. This delivery convention does not change Work's generic item lifecycle.
- Keep acceptance criteria and release checklists in ordinary Markdown. Work does not parse or rewrite those sections.
- Keep ChatGPT conversation links out of repository files.

## Pull requests

- Start each implementation item from current `main` using
  `feature/w-<short-id>-<description>`. Open one bounded pull request per item
  against `main` and use a Conventional Commit title suitable for squash merge.
- Draft the PR from [the template](.github/PULL_REQUEST_TEMPLATE.md). Preserve
  its sections and cite the canonical `.work/items/` path, relevant Mara IDs,
  and verification evidence that actually exists. Use `N/A` when appropriate.
- Squash merge by default. Use rebase merge only when preserving multiple
  independently useful commits that already follow the commit convention.
- When the user intends to publish or continue an item PR, the main session
  uses the [Work PR flow](.agents/skills/work-pr-flow/SKILL.md) and delegates
  GitHub operations to the [PR manager](.codex/agents/work_pr_manager.toml).
  The PR manager does not use the main-session skill. Implementation, tests,
  commits, local Work-item edits, and finding classification stay with the
  main session. The PR manager reports evidence and never merges on its own.

## Bootstrap and releases

- The baseline on `main` contains no CI/release workflow files or generated changelog. Add initial workflow files in the first Rust-foundation feature branch.
- CI should check PRs targeting `main` and pushes to `main`. Validate Mara knowledge using Mara, separately from the future Work executable.
- Release preparation is a separate Work item and PR. Generate `CHANGELOG.md` only for an intentional release candidate; with the planned main-plus-changelog trigger, adding it can start release verification.
- Build and verify one captured main revision before protected publication. Configure the GitHub release environment and any registry trust before publishing; workflow YAML alone does not establish those settings.
- Package names, supported hosts, licensing, and the first version must be explicitly settled before publication. Do not imply that Mara's distribution choices automatically apply to Work.

## Commits

- Follow a repository-specific convention if one is documented. Otherwise use Conventional Commits: `<type>(<scope>): <imperative description>` or `<type>: <imperative description>`.
- Use an established scope when the repository has clear modules or packages; do not invent one. Keep the description concise and omit a trailing period.
- Mark breaking changes with `!` and explain migration impact in a `BREAKING CHANGE:` footer when useful.
