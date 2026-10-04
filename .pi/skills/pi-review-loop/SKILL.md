---
name: pi-review-loop
description: Run an independent local Pi review against a supplied base, triage each finding, fix required defects, and repeat until no blockers remain. Use when an implementing worker's task requires this loop before handoff or the user explicitly requests it. The implementer owns triage and fixes; a reviewer must not recursively invoke this loop.
compatibility: Requires Bash 4+, Git, jq, GNU coreutils, Linux findmnt, Pi with openai/gpt-6.1-sol and the fast-mode extension, and the calling Pi session's PI_SESSION_ID. Provider access is needed for a live review.
---

# Local Pi review loop

The implementing agent owns triage and fixes; the fresh reviewer only reports
findings. Use [triage-findings](../triage-findings/SKILL.md) after each completed
review, including combined-state reviews owned by an integration coordinator.
Follow repository [AGENTS.md](../../../AGENTS.md), applicable project contracts
and commit rules. This does not authorize publication, integration, other agents or
product decisions. Honor the assignment's communication boundaries; ask and
report directly to the user when coordinator communication is prohibited.

## Run one review

From the checkout being reviewed, invoke [scripts/review.sh](scripts/review.sh)
with the full immutable commit SHA of the user-supplied local base. The executable
uses Bash through its shebang, so it can also be called from Fish:

```sh
.pi/skills/pi-review-loop/scripts/review.sh <base-sha>
```

Before launch, use the triage skill to inspect relevant accepted backlog entries
and capture a small plain-Markdown snapshot outside the frozen checkout:

```sh
.pi/skills/pi-review-loop/scripts/review.sh <base-sha> --known-findings /absolute/evidence/known-findings.md
```

The optional file must be nonempty/readable. The launcher copies its bytes and
records its source/hash before supplying it as deduplication data. Refresh it
for each review; do not pass previous raw reviews, required-fix instructions or
a desired verdict. Without the option the prompt explicitly states that no
backlog snapshot was supplied; it does not claim the tracker was empty.
Do not change the checkout or backlog while the reviewer is running.

Resolve the supplied ref immediately before each iteration. Never guess the
base or substitute a remote-tracking ref. The script computes its merge base
with HEAD and records both the supplied SHA and the actual diff baseline.
Run directly unless the user requests Herdr execution; then follow the installed
Herdr skill and run this same command in an explicitly identified shell pane,
preserving focus. Do not create another implementing or PR-manager agent.

The launcher resolves bundled assets relative to itself:

- [assets/reviewer.md](assets/reviewer.md): built-in reviewer instructions.
- [assets/reviewer-prompt.md](assets/reviewer-prompt.md): scope template.

It starts one fresh Pi process with `openai/gpt-6.1-sol`, `xhigh`, `--fast`,
appended reviewer instructions and the substituted scope. It does not resume a
session, send follow-ups, fix code or automatically repeat review. Every Pi call,
including preflight, uses `--no-approve` to skip trust-gated project configuration
and resources, including this review skill. Global resources, repository context
files such as `AGENTS.md`, and the explicitly supplied reviewer prompt remain.
This is not filesystem isolation; Pi also still reads project `sessionDir` before
trust resolution. Keep the no-recursion instruction. Do not substitute another
profile, enable project resources or omit fast mode if unavailable.

Storage follows the global Pi policy using the calling session's `PI_SESSION_ID`:
XDG cache `pi/scratch/<session-id>/` for temporary files and the reusable Cargo
target, XDG state `pi/verification/<session-id>/pi-review.<unique>/` for evidence.
The script rejects RAM-backed roots. It prints the evidence directory before
launch and a `REVIEW_DONE` marker afterward. The child borrows scratch; the
calling session owns cleanup. No environment setup or inline launch scripting
is needed in the assignment.

## Inspect, triage, fix, repeat

1. Complete implementation/checks and freeze HEAD and all working-tree inputs
   during review. The launcher records before/after HEAD, status, tracked diff
   and untracked-content hashes. Any checkout change invalidates the result.
2. Wait for completion and inspect `events.jsonl`, `stderr.log`, `context.json`,
   `command.txt`, `prompt.txt`, any retained known-findings snapshot,
   `before.json`, `after.json` and `status.json`. Exit zero means only successful
   process completion with an unchanged checkout. A timeout does not prove the
   reviewer stopped: inspect actual process state before retrying; never launch
   a duplicate review.
3. Parse the final completed assistant `message_end.message` findings JSON.
   Require successful process completion, `agent_settled`, final
   `stopReason: "stop"`, the requested model/effort/fast-mode evidence and an
   unchanged checkout. Capture the reviewer session ID. Silence, malformed
   output, partial progress, errors or exit zero alone do not pass.
4. Load [triage-findings](../triage-findings/SKILL.md) and disposition every
   finding as fix now, backlog, dismiss or ask the user. Preserve the raw verdict.
   Assess contrary overall explanations and relevant known deferrals too;
   an empty findings array alone cannot override an unresolved blocking reason.
   Record supported rationale and actual backlog destinations in delivery
   evidence; no automatic priority or happy-path filter replaces assessment.
5. Fix all required findings within authorized scope, verify and commit fixes,
   then run a fresh independent review against the newly resolved logical base.
   Do not relay prior raw reviews, fix requests or desired verdicts. Supply only
   relevant accepted deferrals through the captured known-findings option.
   No arbitrary iteration cap replaces resolving required fixes. Ask or report
   blocked for unresolved contracts, authorization or required remedies.
6. Stop when a completed review has no unresolved required fix or clarification,
   all findings have dispositions, authorized backlog recording succeeds, and
   applicable verification passes. Do not repeat solely to eliminate supported
   backlog/dismissed findings. Triage-only delivery-record edits do not transfer
   evidence to a new SHA; retain the actual reviewed/tested identities and
   separately verify any changed code, contract or integration inputs.
7. Base-ref advancement is normal. Resolve the same logical ref anew each
   iteration; do not restart or merge/rebase merely because it advances.
   Movement alone does not invalidate the recorded immutable review baseline.
   Checkout changes do; integration needs verification of its actual combined
   tree and the same triage policy, not just a worker's verdict.
8. Report the logical base, supplied/actual baseline SHAs, reviewed HEAD, dirty
   state, iterations, dispositions, checks and evidence paths. **Clean review**
   requires `findings: []` and `overall_correctness: "patch is correct"` from the
   completed independent reviewer. Otherwise say **accepted after triage** only
   under step 6, listing backlog IDs/entries, dismissal reasons and any contrary
   raw verdict with its supported assessment. Do not rewrite the review or
   describe deferred bugs as fixed. Missing evidence is blocked/pending, not
   acceptance. Preserve needed evidence and clean caller-owned disposable
   scratch after dependent processes exit; honor separate delivery gates.
