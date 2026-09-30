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

Implement shared plain-file storage under the resolved Git common directory, with separate entity folders, a stable short-lived OS coordination lock, safe per-file publication and disposable in-memory views. Keep selected-checkout durable files authoritative. Define versioned store metadata, initialization/loss detection, safe path handling, lock ordering, error categories, and explicit recovery before coding. Entity-specific claim/run/session/handoff operations remain in their bounded items.

The 2026-09-30 file-storage decision supersedes the paused SQLite implementation direction. Existing implementation work is not evidence that this revised acceptance passes; do not resume or publish that implementation as the current contract.

Backup creation and restoring old backups are deferred. This foundation reports damaged or missing state and requires explicit action before recreation, with loss reporting and preservation of surviving context. It does not implement a backup/restore command or validate future claim/run envelopes as a prerequisite.

## Acceptance criteria

- Linked worktrees resolve the same shared entity folders; branch switching or feature-worktree removal does not remove shared operational files or leak durable completion between checkout views. No database or all-entity snapshot is required.
- A selected checkout reloads changed files and gives correct graph readiness; rebuilding a disposable view preserves authoritative entity files byte-for-byte.
- Independent processes serialize mutations through the same stable lock inode; process exit releases the lock. Single-file publication and interrupted initialization/format changes retain explicit recovery context.
- CLI/MCP distinguish contention, invalid formats, missing/corrupt state and permission/I/O failures. Available file inspection/readiness reports storage warnings; storage inspection exposes coordination unavailable for uninitialized, damaged, unreadable or incomplete storage. Tests exercise this lower-level status without a claim command. Claim acquisition consumes and enforces the status in w-299765f4. Tests cover restart, divergent views, unsafe paths and recovery without silently resetting ownership.

## Mara contracts

`ADR-FILE-STATE`, `DES-SHARED-FILES`, `DES-ENTITY-LIFECYCLES`, `DES-FILE-COORDINATION`, `VER-FILE-COORDINATION`, `REQ-SIDE-STATE`, `REQ-WORKTREE-VIEWS`, `REQ-INDEX-REBUILD`, `RISK-DIVERGENT-VIEWS`. Resolve any unsettled operation or storage details in [open questions](../../docs/open-questions.mara.md) and update canonical Mara knowledge before implementation.
