---
format_version: 1
id: "661da1f2e8fd4ad99ddbe041ffe9e3b0"
title: "Build shared coordination storage and view reconciliation"
completion: manual
state: open
priority: 2
parent: "fba5815b224f4c74a194ec2be6dd5d90"
model: "gpt-6-sol"
thinking: "high"
---

## Scope

Implement the Git-common-dir SQLite side store for operational state and derived indexes while keeping each selected checkout's durable item files authoritative. Define schema, migration, backup/recovery, and cache reconciliation before coding.

## Acceptance criteria

- Linked worktrees share one coordination store without leaking one checkout's durable state into another.
- A selected checkout reloads changed files and gives correct readiness despite an old index or branch switch.
- Interrupted migration or index rebuild preserves claims and other non-derivable state; recovery is explicit.
- CLI and MCP expose the applicable inspection or repair path with deterministic diagnostics; tests cover divergent worktree views and database restart.

## Mara contracts

`DES-SHARED-SQLITE`, `REQ-SIDE-STATE`, `REQ-WORKTREE-VIEWS`, `REQ-INDEX-REBUILD`, `RISK-DIVERGENT-VIEWS`. Resolve any unsettled operation or storage details in [open questions](../../docs/open-questions.mara.md) and update canonical Mara knowledge before implementation.
