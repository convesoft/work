---
description: Inspect a Work pull request without publishing or changing it
argument-hint: "[PR number or URL]"
---

Inspect PR ${@:-for the current branch} using gh and report a read-only status
snapshot. Follow AGENTS.md and the inspection guidance in
.pi/skills/work-pr-flow-pi/SKILL.md, resolving that path from the repository root.

Resolve the repository and PR explicitly. Report the current head SHA, required
CI and failed-job links, automatic-review state and reviewed SHA, unresolved
current-scope or critical findings, and the next action. Fetch relevant comment
and thread pages, including the current body of edited review summaries. If
review provenance is unavailable or stale, report pending/unknown, not clean.

Do not push, open or edit a PR, post replies, resolve threads, merge, mutate Work
items, start continuous monitoring, or create Herdr panes. If no PR exists for
the current branch, report that; do not create one.
