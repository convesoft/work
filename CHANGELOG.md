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
