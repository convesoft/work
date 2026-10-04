# Delivery and release conventions

## Ownership

The Mara corpus owns durable product goals, scenarios, requirements, designs, decisions, and verification contracts. Work item files under `.work/items/` own implementation scope, task-specific acceptance criteria, sequencing, assignment, and progress. GitHub owns PRs, review discussion, CI results, merge evidence, and published releases. Product meaning must remain understandable without access to a delivery service.

Delivery tasks use [the Work item format](item-format.mara.md); they are not Mara items. An accepted Mara item establishes knowledge, not implementation or successful execution. Actual verification evidence must identify what was checked and its limitations.

## Repository bootstrap

Prepare an initial `main` snapshot containing the corpus, backlog, agent instructions, README, and ignore rules. Do not include `.github/workflows/` or `CHANGELOG.md` in that snapshot. Remote repository creation, visibility, publication, and server-side settings are separate from preparing this local baseline.

The [first implementation item](../.work/items/87b8795f934049c2acebaac42e81664d.md) starts on a dedicated branch and adds a buildable Rust foundation together with the initial CI/release infrastructure. Pin the toolchain, commit Cargo.lock for the executable, and document commands that actually work. CI checks formatting, lint, tests, and Mara schema/project validity. Work's executable is not a substitute for the Mara validator.

Configure CI for PRs targeting `main` and pushes to `main`. Validate the first feature PR, then configure its check as required for subsequent merges. A workflow's presence does not prove that a remote check, branch rule, environment protection, or registry trust is configured.

## Implementation delivery

Both delivery modes are supported. Select the mode before starting new work; keep existing branches unless explicitly changing their placement.

### Individual issue

Start the item branch from current `main` using `feature/w-<short-id>-<description>`. Open one bounded PR against `main` and squash merge it.

### Bounded local scope

1. Use one manual Work item to own a bounded version or milestone scope. Record the repository, intended outcome, included canonical Work IDs, relevant Mara IDs, integration order, and final acceptance checks. Reuse the release-preparation owner when it already owns this scope; a version name is not a release or publication approval.
2. Start a short-lived scope branch from current `main` using `feature/w-<short-id>-<description>` with the scope owner's ID. Start each included issue branch from the current scope tip using the same convention with that issue's ID. Do not introduce long-lived development or release branches.
3. Implement, verify, and locally review each issue against its scope base before integration. Follow the repository's review rules; run the independent Pi review loop only when explicitly required. Squash integration is not review or CI.
4. With a clean scope checkout, run `git merge --squash <issue-branch>`, inspect the staged result, and commit one logical Conventional Commit for that issue. Identify its canonical Work ID in the commit body or footer. This creates no merge commit and does not record merge ancestry. Verify the integrated result before starting dependent work; after integration, branch subsequent work from the new scope tip rather than reusing the old issue branch.
5. Record the declared target, integrated commit, acceptance and review evidence in each issue. Close it after verified integration into the scope branch under the completion gates below. Record remaining scope delivery work in the scope owner.
6. Run the final combined acceptance checks on the exact scope head. Open one scope PR against `main`, identifying the owner and all included items, relevant Mara IDs, actual verification evidence, and limitations. Every retained commit must be independently useful and follow the commit convention. Rebase merge the scope PR to preserve per-issue commits in linear `main` history; do not squash the entire scope or use a merge commit.

If `main` advances, verify the updated integration result before merging. Updating or rewriting an existing branch requires explicit authorization; this workflow does not authorize incidental rebases or force-pushes. A changed PR head requires fresh applicable checks and review. GitHub must allow rebase merging for scope PRs; unavailable merge methods are a blocker, not permission to silently choose another method.

### Finding disposition

Use [finding triage](../.pi/skills/triage-findings/SKILL.md) for issue, cloud and combined-tree findings; run the [Pi review loop](../.pi/skills/pi-review-loop/SKILL.md) when required. The implementer or integration coordinator records supported fix-now, backlog, dismiss or ask-user dispositions without rewriting the independent review.

No current-acceptance or critical defect is waived by deferral. Required fixes/clarifications and applicable verification must resolve before acceptance. Report a clean review only from completed no-finding/correct evidence; otherwise report accepted after triage with actual backlog references and disposition rationale. Preserve the reviewed/tested identities. This policy does not replace required approval, CI, integration checks or publication authorization.

### Evidence and completion

Work IDs remain stable across Git history rewriting. Local squashing and GitHub rebase merging change commit identities. Evidence records the actual tested revision and scope; do not relabel old evidence as testing a new SHA. Record the final `main` integration revision and run the applicable final-`main` checks. Release verification still targets one captured `main` revision.

Close after verified integration into the declared delivery target: acceptance must pass, required review for that target must be complete, and the integrated result must be verified. Declare the target before implementation and record its branch and integrated revision in the item.

| Item | Declared delivery target and closure gate |
| --- | --- |
| Individual issue | `main`; acceptance, required cloud review/CI, merge and integration verification |
| Issue within a bounded scope | Scope branch; acceptance, required local review, squash integration and integration verification |
| Scope owner | `main`; combined acceptance, required cloud review/CI, scope PR merge and integration verification |
| Release owner | Its declared integration target plus all release/publication acceptance criteria |

Closed scope issues do not wait for the enclosing scope PR. The scope owner tracks remaining combined checks, cloud review, CI and delivery to `main`. If later review invalidates an issue's acceptance, reopen that issue; new scope-level integration defects may have their own items. Aggregate completion remains derived from children; a manual owner is not a derived aggregate.

Pushes, PR merges, branch/history rewrites, and release publication require their respective authorization. Adoption of this workflow does not itself authorize them.

These are repository delivery conventions, not changes to generic Work items under [[DES-LIFECYCLE]] or the product's execution-scope contracts. Never mark the live backlog complete merely to exercise acceptance tests; use disposable copies.

## Release preparation

The [release-preparation item](../.work/items/74c22e40cbc249229f86e355a663a942.md) follows the first usable CLI/MCP milestone. One manual item owns release preparation and publication follow-through; reuse the owner of the intended release. It prepares an independently reviewed candidate through either delivery mode. In scope mode, identify and review release preparation within the scope PR. An early alpha may contain just that usable slice, with its limitations explicit; it must not claim all of [[ADR-INITIAL-SCOPE]] is implemented.

The intended workflow follows Mara's separation between verification and protected publication. A push to `main` changing `CHANGELOG.md` starts release verification. Keep the changelog out of the foundation PR; generate it from Conventional Commit history with git-cliff in the individual release-preparation PR or its enclosing scope PR, alongside the manually selected version. Apply the `release` label to the PR containing release preparation as a maintainer convention. Like Mara, the workflow does not inspect that label: the automated trigger is the `main` push changing `CHANGELOG.md`, not a label event.

Capture one main commit for source checks, artifact builds, package/install inspection, and real CLI/MCP smoke tests on every declared supported host. Publication depends on successful verification and approval through the protected release environment. Use an annotated immutable version tag at that commit and publish a draft GitHub release only after distribution checks succeed. A failed staged publication may have created external artifacts; inspect completed stages and retry only the same revision and matching artifacts.

[[ADR-WORK-DISTRIBUTION]] settles Work's npm package names, three supported hosts, and the first candidate version and license: `0.1.0-alpha.1` under `MIT OR Apache-2.0`. The npm trusted publisher must identify `convesoft/work` and `.github/workflows/release.yml`, with no npm environment-name restriction. GitHub's protected `release` environment is configured separately. Verify external registry trust and environment protection in the release-preparation item; authored workflow files are not evidence of those settings. Publish native packages before the dispatcher, verify existing package digests on retries, and check the public install before publishing the GitHub release. Do not claim other-host success from local testing.

The foundation item owns workflow implementation. The release item owns candidate preparation, confirmation of external settings, publication, and evidence. The candidate provides the selected Cargo version and license, full license texts, executable `scripts/smoke-packaged.sh` with real packaged CLI/MCP checks, and a generated `CHANGELOG.md`. The publication job depends on successful source verification and all supported-host builds, then requires approval through the protected GitHub `release` environment. There is no additional repository-variable readiness switch. Verify the actual environment protection and npm trusted-publisher settings before approving publication; removing an extra switch does not establish those settings. Merging and pushing the preparation change starts verification, but a PR label or merge does not by itself approve publication.
