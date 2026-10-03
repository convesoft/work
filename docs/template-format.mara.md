:::mara design DES-TEMPLATE-FORMAT
:mid: 01M3Q09ZXR7G7P4Y13K99PQVPW
:title: Define version-1 YAML templates and symbolic preview
:status: accepted
:kind: data
:satisfies: REQ-GRAPH-TEMPLATES
:satisfies: REQ-EXECUTOR-HINTS

Version 1 templates are UTF-8 YAML files directly under `.work/templates/<name>.yaml`. The top-level mapping contains `format_version: 1`, `name` matching the filename stem, optional `parameters` and `existing` name lists, optional `defaults` with `model` and `thinking`, a nonempty `items` list, and optional `edges`. Parameter, existing-binding, and item-key names use distinct, unique `[a-z][a-z0-9_]*` identifiers. Reject unknown or duplicate YAML keys, unsupported versions, and unsupported fields.

Each item has a unique `key` and required `title`. Optional `body` defaults to empty Markdown; optional `completion`, `priority`, `labels`, `model`, and `thinking` follow [[DES-ITEM-FORMAT]]. Omitted completion is `manual`, omitted priority is 2, new manual items start open, and aggregates omit recorded state. Only model and thinking receive template defaults, independently; an explicit item hint wins. Preview returns each effective hint and whether it came from the item or template default. Hints never select or launch an executor.

The caller supplies a full canonical ID for the existing root item, one full canonical ID for each declared `existing` binding, and exactly one text value for each declared parameter. `{{name}}` tokens are allowed only in item title, body, labels, model, thinking, and the two hint defaults. Render each token once from its declared value. Reject missing or extra values, unused parameter declarations, undeclared tokens, and malformed or unmatched `{{` or `}}`. Insert supplied text literally without evaluating replacement text again. Validate all rendered item fields.

Each edge has `from`, `kind`, and `to`. Endpoints use `local:<key>` for a prospective item, `root` for the selected existing root, or `existing:<name>` for a declared binding. Structural fields and references do not interpolate. Kinds are the four relations in [[DES-RELATION-MODEL]], in their established source-to-target direction. Resolve all endpoints and validate the complete prospective graph against the selected checkout's current durable view. Once [[DES-EPHEMERAL-LAYOUT]] defines the run item format, the same validation includes retained run items. Reject missing targets, duplicate semantic edges, and invalid lifecycle graphs before returning a preview.

Discovery, validation, and preview are read-only CLI and MCP operations over one shared core. Preview returns the rendered item metadata and full body keyed by local key, resolved parameter values, effective hint provenance, and the complete sorted edge list. For fixed template bytes, inputs, and selected source view, its symbolic graph is deterministic. Preview creates no run, items, permanent IDs, or claims. Later template expansion assigns permanent IDs when it creates files; it does not record a template application entity. See [[DES-TEMPLATE-RUNS]].

The implemented version-1 preview format above has no per-item material/wisp selector. [[DES-RUN-API]] defines version 2 with per-item `persistence: material|wisp`, default material, and optional root when no edge uses it. Material-only planning needs no run. The development parser and expansion support both versions; the published alpha has no template operations. `tests/templates.rs`, `tests/run_execution.rs` and the connected beta harness exercise compatibility, preview and publication. Consult actual check results rather than treating the accepted format as passing evidence.
:::
