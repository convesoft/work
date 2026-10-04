---
name: pi-review-loop
description: Run an independent local Pi review against a supplied base, fix every finding, and repeat until no findings remain. Use when an implementing worker's task requires this loop after implementation and before handoff, or the user explicitly requests it. The implementing agent owns fixes. A reviewer producing findings must not recursively invoke this loop.
compatibility: Requires Bash 4+, Git, jq, GNU coreutils, Linux findmnt, Pi with openai/gpt-6.1-sol and the fast-mode extension, and the calling Pi session's PI_SESSION_ID. Provider access is needed for a live review.
---

# Local Pi review loop

The implementing agent owns this loop; the fresh reviewer only reports findings.
Follow repository [AGENTS.md](../../../AGENTS.md), applicable Mara contracts and
commit rules. This does not authorize publication, integration, other agents or
product decisions. Honor the assignment's communication boundaries; ask and
report directly to the user when coordinator communication is prohibited.

## Run one review

From the checkout being reviewed, invoke [scripts/review.sh](scripts/review.sh)
with the full immutable commit SHA of the user-supplied local base. The executable
uses Bash through its shebang, so it can also be called from Fish:

```sh
.pi/skills/pi-review-loop/scripts/review.sh <base-sha>
```

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

## Inspect, fix, repeat

1. Complete implementation/checks and freeze HEAD and all working-tree inputs
   during review. The launcher records before/after HEAD, status, tracked diff
   and untracked-content hashes. Any checkout change invalidates the result.
2. Wait for completion and inspect `events.jsonl`, `stderr.log`, `context.json`,
   `command.txt`, `before.json`, `after.json` and `status.json`. Exit zero means
   only successful process completion with an unchanged checkout, not a clean
   review. A timeout does not prove the reviewer stopped: inspect actual process
   state before retrying, and never start duplicate reviews.
3. Read the final completed assistant `message_end.message` text from the JSONL
   stream and parse its findings JSON according to the reviewer asset. Require
   successful process completion, `agent_settled`, final `stopReason: "stop"`,
   the requested model/effort/fast-mode evidence and an unchanged checkout.
   Clean means `findings: []` and `overall_correctness: "patch is correct"`.
   Silence, malformed output, partial progress, errors or exit zero alone do
   not pass. Capture the reviewer session ID from the event header.
4. Fix every finding in the supplied-base diff, verify and commit fixes, then
   invoke the script again. Do not relay previous findings to the fresh reviewer.
   No arbitrary iteration cap or ordinary backlog/dismissal filtering replaces
   this opted-in loop. If a finding cannot be resolved, contradicts canonical
   knowledge or needs a product decision, ask the user or report blocked.
5. Base-ref advancement is normal. Resolve the same logical ref anew for each
   iteration; do not restart implementation or merge/rebase merely because it
   advances. Movement alone does not invalidate evidence against a recorded
   immutable baseline. Worker checkout changes do; integration must separately
   verify compatibility with its latest combined tree.
6. Report the logical base ref, supplied/review baseline SHAs, reviewed HEAD,
   dirty state, iterations/fixes, actual checks and final conclusion with evidence
   paths. A review execution limitation is not a passing test: record it and
   obtain required execution evidence separately. Preserve needed evidence and
   clean up owned disposable scratch under the global Pi policy when dependent
   processes exit. If stopped early, report blocked or paused, never complete.
