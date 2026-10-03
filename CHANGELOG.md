## [0.1.0-beta.1]

This beta candidate adds shared file storage, exclusive claims and atomic scoped claim-next, parameterized templates, mixed material/wisp runs, receiver-scoped handoffs, reusable workspaces and named sessions, explicit external workspace cleanup reporting, and caller-authored root digests or ephemeral discard through the CLI and MCP.

Work does not execute agents, manage physical worktrees, or perform PR/CI actions. Backup/restore, automatic expiry/pruning, remote coordination and the accepted storage-recovery follow-up remain deferred. Template expansion may leave inspectable partial results; explicit discard can retain handoffs with missing receivers. See docs/using-work.md for retention, retry and platform limits. Candidate preparation is not publication or final-main/all-host verification.

### Documentation
- *(storage)* Define plain-file entity storage and lifecycles (#13)
- *(work)* Close storage and template delivery items
- Settle beta execution contracts and worker handoffs (#16)

### Features
- *(templates)* Preview parameterized work graphs
- *(storage)* Implement shared file coordination foundation
- *(claims)* Coordinate exclusive ownership across worktrees (#17)
- *(runs)* Expand templates into material items and wisps (#18)
- Persist and deliver receiver-scoped handoffs
- Integrate reusable workspaces and run-scoped sessions
- Integrate scoped selection and atomic claim-next
- Integrate explicit workspace cleanup reporting
- Integrate run digests and explicit discard

### Maintenance
- *(work)* Record merged runs completion (#19)
- Support Pi and Codex PR workflows
- Use global MCP configuration
- Align review policy with Mara
- *(work)* Record locally integrated handoffs completion
- *(work)* Record integrated selection and context completion
- *(work)* Record integrated workspace cleanup completion
- *(work)* Record integrated digest and discard completion
- *(work)* Record integrated beta verification completion

### Testing
- Integrate complete beta workflow verification
## [0.1.0-alpha.1]

This alpha delivers the durable Work item graph through the CLI and MCP server. It supports item creation, inspection, updates, relationships, readiness, and closure in Git worktrees.

Claims, templates, temporary runs, sessions, workspaces, handoffs, and run finalization are planned for later candidates.

### Features
- Establish Rust foundation and Git discovery (#1)
- *(items)* Load version-1 work items (#2)
- *(graph)* Evaluate item readiness and relationships (#3)
- *(operations)* Implement safe durable item operations
- *(cli)* Expose durable item operations as JSON
- *(mcp)* Expose durable item operations over stdio (#6)

### Maintenance
- Establish project baseline and bootstrap backlog

### Testing
- *(items)* Verify bootstrap adoption and document usage (#7)
