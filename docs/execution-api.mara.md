:::mara design DES-EXECUTION-IO
:mid: 01M3TWR24EYHGH9N2B5AMHC7NW
:title: Share execution file IO, resolved views and transport rules
:status: accepted
:kind: interface
:satisfies: REQ-WORKTREE-VIEWS
:satisfies: REQ-CLI-MCP-PARITY
:satisfies: REQ-CLAIM-EXCLUSION

## Scope and common values

This is the beta interface for the later execution slices, not a claim of implementation. The storage foundation stays as implemented in [[DES-STORE-FOUNDATION]]. All operations use the discovered Git common directory and its current store identity/generation. They do not initialize or recreate storage implicitly.

Entity IDs (claim, run, workspace, handoff and session-record IDs) use generated UUIDv4, encoded as 32 lowercase hex characters, as item IDs do. Authored references store full IDs. APIs return full IDs; human abbreviations may use the existing unambiguous item resolver, while entity references require full IDs in beta. Creation checks collisions and never overwrites another entity. Timestamp fields are UTC RFC3339 strings produced by Work; timestamp order never selects ownership. An external session is exactly `{namespace: string, id: string}`; both nonempty opaque UTF-8 strings, compared case-sensitively. Actor is a nonempty caller-supplied label, not an authenticated OS principal.

New YAML envelopes use `format_version: 1`, one mapping, exact keys and types, no duplicate/unknown keys, tags, aliases or anchors. Fields listed with `?` are optional and omitted, not null; arrays may be empty unless stated otherwise. ID arrays are unique and sorted on write. A future version yields `unsupported_format` before interpreting version-1 fields. Existing item/template contracts retain their own framing and validation. UTF-8 paths in CLI inputs use normal path arguments. Persist and emit absolute discovered paths using the existing percent-encoded Unix-path representation, decoded once before filesystem use. No arbitrary relative path is replayed from entity data.

## Coordinator-owned IO boundary

The main implementation session owns `src/core/coordination.rs`, `src/core/context.rs`, module exports and necessary small storage/operations integration edits. Worker modules must consume this boundary; they must not copy the foundation lock implementation.

Expose a crate-internal `CoordinationGuard` acquired in shared/read or exclusive/write mode. Acquire the stable common-directory lock without creating it, inspect foundation availability under that same held lock, and retain it through reload/validation/publication. Do not call a public storage method that reacquires the same lock. Entity mutations require coordination available; standalone durable-only reads retain the existing warning behavior when it is unavailable. They must not claim ownership or pretend missing runtime context was successfully resolved.

The guard provides validated entity read, create-new, expected-source replacement and deletion, plus current `StoreMetadata`. Reuse existing descriptor-bound IO and source-conflict behavior through a small crate-private wrapper; do not build a new generic transaction/recovery engine. Take checkout-local mutation locks after the shared lock, with multiple checkout paths in lexical canonical-path order. Never re-enter a checkout lock through the old public operations facade while holding it. The main session provides lock-held durable-file primitives for the coordinator; workers operate on supplied guards and snapshots.

Single files retain existing publication and sync rules. Entity inspection never writes. Lists are complete and sorted in beta; no pagination or background polling. Unknown/malformed files in an entity directory are diagnostics, never an empty ownership set. No filesystem-hardening expansion beyond the implemented foundation is required by this item.

## Resolved item view

`ResolvedView` holds the selected project, items/graph, source worktree and path per item, source fingerprints, workspace bindings and optional current run context. Build it from the selected checkout's durable catalog, overlay each explicitly bound material ID with the entire file in its bound workspace, and include live wisps from retained runs. Bound IDs absent in the selected checkout are included. One physical source per item is selected before graph evaluation; never mix header/body/relations from different versions.

A material binding is `workspaces/items/<item-id>.yaml`: exact fields `format_version, store_id, recovery_generation, item_id, workspace_id`. This is the location reference, not a copied item state. A referenced workspace must resolve to a linked working checkout of the same Git common directory and contain the matching item. Missing/malformed/ambiguous sources are diagnostics and block graph-dependent mutations; do not fall back to main. Wisp IDs must be unique across run directories and material IDs. They resolve from their owning run, not through a material binding.

Normal inspection/readiness and mutations use this view and report `source_worktree` (null for a wisp), `path`, `persistence: material|wisp`, and `run_id` (wisp owner, otherwise current membership or null). Existing recorded/derived state fields retain their meanings and are calculated from these actual files. Explicit `item list|inspect|ready|diagnose --view checkout` and MCP `view: "checkout"` select the old durable-only physical view for inspection; default is `resolved`. Raw inspect/repair remain explicitly physical checkout operations and must say so in help. They refuse a competing active claim once claim integration ships.

A material mutation writes the resolved item file immediately. Binding creation/rebinding validates the destination first and refuses active claims on that item. Retain bindings after completion, release and run disposal. Rebind to a surviving checkout explicitly after merge or relocation; no automatic Git action or branch merge is performed. Worktree default/override selection is defined in [[DES-CONTEXT-API]].

## Transport and errors

Keep CLI JSON `{ok:true,result:...}` / `{ok:false,error:...}` and MCP's matching structured domain payload. Add fields to existing item results rather than changing their established fields. Optional context fields are null in JSON when absent. Human output displays ownership, source and storage warnings. New mutation responses always include `changed`; list operations return a named array, sorted by ID unless priority ordering is specified.

New domain codes: `claim_conflict, stale_claim, not_ready, run_conflict, run_not_current, workspace_busy, reference_blocked, source_unavailable` exit 5; `not_found` exit 3; malformed entity `invalid_format`, unsupported version, identity/generation mismatch and unavailable storage exit 4; invalid arguments exit 2; permission/IO exit 1. Existing storage lock contention remains `storage_busy` exit 5. Preserve foundation error fields and existing durable codes/exits. Errors include code/message plus applicable IDs, paths, diagnostics and blocker IDs. Multi-file errors also return `partial` with known `created`, `updated`, `deleted` records (`id,path`), and `uncertain_paths`; `publication: not_published|possible|published` describes the last attempted write. Do not relabel an underlying IO error as success.

Claim-aware mutation requests accept `authorization` as an array of `{claim_id, session:{namespace,id}}`, default empty; CLI repeats `--authorize JSON` with one such object per occurrence; JSON preserves opaque namespace/session text without inventing a delimiter encoding. The core validates the pair for each claimed source item being changed. An unclaimed item can be changed without a pair; supplying a stale pair always fails. This is cooperative local ownership, not secret authentication. Explicit controller reassignment/release uses the separate recovery interface in [[DES-CLAIM-API]]. Claims never prevent reading. Unrelated graph targets are not owner-authorized merely because an edge points at them; authorize the files actually modified.
:::
