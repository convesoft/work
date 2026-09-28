# Delivery and release conventions

## Ownership

The Mara corpus owns durable product goals, scenarios, requirements, designs, decisions, and verification contracts. Work item files under `.work/items/` own implementation scope, task-specific acceptance criteria, sequencing, assignment, and progress. GitHub owns PRs, review discussion, CI results, merge evidence, and published releases. Product meaning must remain understandable without access to a delivery service.

Delivery tasks use [the Work item format](item-format.mara.md); they are not Mara items. An accepted Mara item establishes knowledge, not implementation or successful execution. Actual verification evidence must identify what was checked and its limitations.

## Repository bootstrap

Prepare an initial `main` snapshot containing the corpus, backlog, agent instructions, README, and ignore rules. Do not include `.github/workflows/` or `CHANGELOG.md` in that snapshot. Remote repository creation, visibility, publication, and server-side settings are separate from preparing this local baseline.

The [first implementation item](../.work/items/87b8795f934049c2acebaac42e81664d.md) starts on a dedicated branch and adds a buildable Rust foundation together with the initial CI/release infrastructure. Pin the toolchain, commit Cargo.lock for the executable, and document commands that actually work. CI checks formatting, lint, tests, and Mara schema/project validity. Work's executable is not a substitute for the Mara validator.

Configure CI for PRs targeting `main` and pushes to `main`. Validate the first feature PR, then configure its check as required for subsequent merges. A workflow's presence does not prove that a remote check, branch rule, environment protection, or registry trust is configured.

## Implementation delivery

Start each item from current `main` on `w-<short-id>-<description>`. Keep one bounded PR per item, identify the canonical Work item, and reference relevant Mara knowledge. Describe the change and actual validation. Use a Conventional Commit PR title and squash merge by default.

Keep implementation items open through implementation, review, and fixes. Mark them done after acceptance passes and the change is merged. This is this repository's delivery policy; generic Work items remain governed by [[DES-LIFECYCLE]]. Aggregate completion remains derived from children. Never mark the live backlog complete merely to exercise acceptance tests; use disposable copies.

## Release preparation

The [release-preparation item](../.work/items/74c22e40cbc249229f86e355a663a942.md) follows the first usable CLI/MCP milestone. It prepares a separately reviewed candidate and owns publication follow-through. An early alpha may contain just that usable slice, with its limitations explicit; it must not claim all of [[ADR-INITIAL-SCOPE]] is implemented.

The intended workflow follows Mara's separation between verification and protected publication. A push to `main` changing `CHANGELOG.md` starts release verification. Keep the changelog out of the foundation PR; generate it from Conventional Commit history with git-cliff in the release-preparation PR, alongside the manually selected version. A release label is organizational metadata, not an authorization check.

Capture one main commit for source checks, artifact builds, package/install inspection, and real CLI/MCP smoke tests on every declared supported host. Publication depends on successful verification and approval through the protected release environment. Use an annotated immutable version tag at that commit and publish a draft GitHub release only after distribution checks succeed. A failed staged publication may have created external artifacts; inspect completed stages and retry only the same revision and matching artifacts.

Choose Work's version, license, package names, installation channels, and supported hosts explicitly. If npm is selected, configure trusted publishing, publish native packages before the dispatcher, verify existing package digests on retries, and check the public install before publishing the GitHub release. Do not copy Mara's package identities or claim other-host success from local testing.

The foundation item owns workflow implementation. The release item owns candidate preparation, confirmation of external settings, publication, and evidence. No release workflow or release execution exists in the baseline.
