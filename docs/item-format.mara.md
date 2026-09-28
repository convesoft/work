:::mara design DES-ITEM-FORMAT
:mid: 01M3KY9FHP1B78E3QJSKVA6VE3
:title: Serialize version-1 work items as YAML frontmatter and opaque Markdown
:status: accepted
:kind: data
:satisfies: REQ-DURABLE-FILES
:satisfies: REQ-OPAQUE-BODIES
:satisfies: REQ-BOOTSTRAP-COMPATIBILITY
:satisfies: REQ-RELATION-STRUCTURE

Version 1 is the durable Work item format used by the bootstrap backlog and the first implementation. Files live directly under `.work/items/<id>.md`; the filename stem equals the full canonical UUIDv4 identity. Temporary items will use the same item envelope in the shared run location, with run manifests specified separately. Storage location determines durability; no `ephemeral` flag is serialized in item frontmatter.

Each file is UTF-8 without a byte-order mark. Its first line is exactly `---`; its first subsequent standalone `---` line ends the YAML header. LF and CRLF delimiters are accepted. Everything after the closing delimiter's line ending is the opaque body, including leading blank lines and the final-newline choice. Writers preserve those body bytes during metadata changes and never parse headings, acceptance criteria, checkboxes, or links.

The header is one YAML mapping with unique string keys. Use the JSON-compatible scalar/sequence/mapping subset of YAML 1.2; disallow aliases, anchors, custom tags, merge keys, duplicate keys, and additional YAML documents. IDs and other strings must decode as strings; quote ambiguous scalars. Invalid UTF-8, malformed headers, unknown format versions, unknown top-level keys, and wrong types produce diagnostics and no writes. Do not coerce a numeric ID or silently drop unknown fields. Header formatting and comments may be normalized by an explicit metadata edit; body bytes may not.

| Field | Type and meaning |
| --- | --- |
| `format_version` | Required integer, exactly `1` |
| `id` | Required string: 32 lowercase hexadecimal characters representing a UUIDv4 |
| `title` | Required nonblank single-line string |
| `completion` | Optional `manual` (default) or `children` |
| `state` | Required `open` or `done` for manual items; prohibited for aggregates |
| `priority` | Optional integer 0–4, default 2; lower numbers are selected first: urgent, high, normal, low, backlog |
| `parent` | Optional single full item ID |
| `depends_on` | Optional list of distinct full item IDs; default empty |
| `related` | Optional list of distinct full item IDs; default empty |
| `discovered_from` | Optional list of distinct full item IDs; default empty |
| `labels` | Optional list of distinct nonblank single-line strings; default empty |
| `model`, `thinking` | Optional nonblank single-line strings interpreted by the external executor |
| `close_reason` | Optional nonblank single-line string, only on a manual item with `state: done`; remove when reopening |

An omitted optional scalar has no value; `null` is not an alternative spelling. Unknown extensions require a future explicit contract rather than silently accepting misspelled control fields. Version changes must be explicit; unsupported versions are never rewritten as version 1. No claims, named sessions, workspaces, run manifests, timestamps, or parsed acceptance fields are required in an item document. They must not be invented to bootstrap durable work.

For `completion: children`, omit both `state` and `close_reason`. Compute effective state from the graph using [[DES-RELATION-MODEL]]: a nonempty set of resolved children yields done, and an empty aggregate stays open. This avoids a stale cached state in versioned files. Aggregate cancellation is not an override of derived state; resolve its required child obligations instead. Manual items retain recorded state and do not reopen automatically.

Persist each relationship once. A child stores `parent`; a dependent stores `depends_on`; a discovered item stores `discovered_from`. A symmetric `related` edge may be authored at either endpoint, but not both. Expose both directions on read without materializing a second assertion. Resolve full IDs across the selected durable view and retained temporary items; reject missing targets, self-relations, repeated semantic edges, multiple parents, and lifecycle cycles/deadlocks. Different relation kinds between the same items remain valid. Markdown body references are opaque text, not graph edges.

Labels are exact case-sensitive values with no automatic hierarchy inheritance or lifecycle effect. Priority and model/thinking hints do not inherit from parents. Templates can render explicit values during instantiation. For equal priority, use canonical ID lexicographic order as a stable selection tie break; author array ordering has no scheduling meaning. Label-filter syntax and other operation schemas remain separate interface design.

A manual item example (the body is illustrative, not parsed):
```markdown
---
format_version: 1
id: "d66b0ba51d2c4a7aa15de40cb3c9d507"
title: "Implement item loading"
completion: manual
state: open
priority: 2
---
Read the existing item files without changing them.

Acceptance criteria are ordinary Markdown written by the caller.
```

The companion [frontmatter schema](schemas/work-item-v1.schema.json) describes decoded header shape. It cannot check delimiter syntax, filename equality, duplicate YAML keys, body preservation, reference resolution, or graph semantics; those checks remain part of this contract.
:::
