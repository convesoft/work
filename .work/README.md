# Work bootstrap backlog

These are real version-1 Work items, authored before the executable exists. They are delivery issues, not Mara specification items.

The file contract is [DES-ITEM-FORMAT](../docs/item-format.mara.md); the [JSON Schema](../docs/schemas/work-item-v1.schema.json) checks decoded frontmatter shape only.

## First delivery

| Order | Item | Purpose |
| --- | --- | --- |
| Aggregate | [w-933a6d82](items/933a6d82674c4a23ab45e810b8319bd2.md) | Deliver the first usable Work item loop through CLI and MCP |
| 1 | [w-87b8795f](items/87b8795f934049c2acebaac42e81664d.md) | Establish the Rust core, Git discovery, and CI/release infrastructure |
| 2 | [w-0fac69ec](items/0fac69ec66004e088ce2b22cc0119bd2.md) | Load version-1 items and preserve their opaque bodies |
| 3 | [w-13ca14c6](items/13ca14c6e5c14b24a5dea551c343bbeb.md) | Evaluate relationships, effective completion, and readiness |
| 4 | [w-cec96d7e](items/cec96d7e174d47c1ab3117968b0bba0f.md) | Implement safe shared operations for durable items |
| 5 | [w-393f86fa](items/393f86fafaf54449944d27da1af0a24b.md) | Expose the durable item loop through CLI and structured JSON |
| 6 | [w-04368710](items/04368710988a405d8e96f85d3bae6b01.md) | Expose the same durable item loop through MCP |
| 7 | [w-e1121992](items/e1121992e0574b4b9b163b85e17ec601.md) | Verify bootstrap adoption and document the first usable loop |
| Release | [w-74c22e40](items/74c22e40cbc249229f86e355a663a942.md) | Prepare and verify the first alpha after the aggregate completes |

The seven manual tasks form a dependency chain under the aggregate. This table is a navigation aid; item files own state and relationships and determine which task is next in the chain.

The release item is a separate manual item depending on the aggregate. It owns the release candidate and generated changelog. The first feature branch owns the initial workflow files. See [delivery conventions](../docs/delivery.mara.md).

## Working before the CLI exists

1. Read the selected item and the Mara contracts referenced in its body.
2. Implement its scope and verify its acceptance criteria.
3. For an implementation item whose acceptance passes and change is merged, change `state: open` to `state: done`; optionally add a single-line `close_reason`. The release item also requires publication evidence.
4. On reopening, set `state: open` and remove `close_reason`. Do not automatically reopen completed downstream work.
5. Keep IDs and filenames stable, use full IDs in relationships, and preserve the Markdown body when editing metadata.
6. Do not write state on the aggregate; its completion is derived from its children.

During bootstrap, serialize manual ownership through the controlling session; these files do not implement claims. The future CLI/MCP must adopt them directly. Run destructive or state-changing acceptance scenarios on disposable copies, not this live backlog.

## Scope

This batch delivers the durable-item graph loop through CLI and MCP. Later slices cover coordination, templates/runs, sessions/workspaces, handoffs, and finalization. They remain part of the product scope in Mara but are not expanded into speculative tickets here.

The prepared Git baseline preceded feature work. The first implementation issue established the Rust foundation and initial workflows. The generated changelog belongs to the separate release item.
