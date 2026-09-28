---
name: work-pr-flow
description: Main-session handoff for publishing or continuing a Work item pull request when the user intends to send work to GitHub.
---

# Work pull request flow

This skill is for the implementation session. The project `work_pr_manager` agent
owns publication, CI/review monitoring, and explicitly directed GitHub
conversation operations. Its instructions live in
`.codex/agents/work_pr_manager.toml`; do not ask it to use this skill. Keep the
main session active during the review loop.

1. Complete and verify the authorized work, then commit it. Give the PR manager
   the canonical Work item path and ID, branch, exact ready-to-push SHA,
   relevant Mara IDs, verification evidence, known limitations, and existing PR
   URL, if any. Select the project `work_pr_manager` agent when Codex CLI
   supports custom agent selection. Otherwise, use a `gpt-6-luna` subagent at
   high reasoning effort with no forked turns; pass this handoff and the agent
   file's `developer_instructions` in its task, and explicitly forbid it from
   using this skill. Do not delegate product decisions, local Work-item edits,
   source edits, verification, commits, or finding classification.
2. When the agent reports a PR URL, include it in the main session's status
   and final handoff. Codex CLI has no task-attachment control; the URL is the
   durable link. Review each finding against `AGENTS.md` and the applicable
   Mara contract. Make and verify accepted fixes in the main session, or
   decide on a supported reply, dismissal, or backlog outcome. Give the agent
   the exact new commit SHA and intended disposition of each finding. The
   agent handles the push and explicitly directed GitHub replies/resolutions.
3. Continue the same agent and PR for subsequent reviewed heads. After each
   push, let it poll every 60 seconds for the automatic review; never request
   a manual `@codex review`. Treat its merge-ready report as evidence to check
   against the Work item acceptance criteria. Merge only when separately
   instructed.
