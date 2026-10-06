# Test migration for bounded hosting, initialization, runs and templates

Owners: hosting `7bbc0619ab46461fa0d5c3b42edbeef1`; initialization `6bf6198a34494a94a30a9ba53ed976a6`. This is delivery/test allocation,
not a change to the accepted beta.2 catalog or a claim that deferred consumers work.

## Active evidence

`cargo test --locked --all-targets` compiles every retained Rust suite and runs
the applicable hosting, material-model and safe-file regressions. The public
`scope_hosting.rs` launcher runs `scope_hosting.mjs` against the just-built
executable: real disposable Git repositories/worktrees and actual CLI/MCP
processes, not mocks. Its prepared v2 execution records prove validation,
routing and byte/identity preservation only.

The existing `items.rs`, `graph.rs`, `operations.rs`, `claims.rs`,
`runs.rs` and `templates.rs` suites remain active. In particular,
`operations.rs` retains close/reopen, explicit repair, malformed/misnamed
sources, unsafe source paths, restrictive modes, changed inodes, no-overwrite
creation, invalid graphs and opaque-body preservation. Direct old core storage/
claim tests exercise their retained model and shared primitives, not public
beta.2 initialization/acquisition/lifecycle acceptance.

Mixed `bootstrap_adoption.rs`, `cli_json.rs`, `mcp_protocol.rs`,
`claims_execution.rs`, `coordination.rs` and `storage.rs` suites retain
active material/public-read, invalid-input, concurrency and safe-file cases.
Eight ordinary material consumer tests in `tests/pending/` are reactivated: CLI/MCP full lifecycle, malformed/immutable repair, authored-backlog closure, healthy-repair refusal, lookup errors and authority isolation. They now call actual ordinary material completion/repair, not external state substitution. The public `onboarding.rs` launcher additionally runs `onboarding.mjs` and `material_completion.mjs`: actual CLI/MCP initialization/status and authoritative material operations, including linked/detached primary layouts, unsupported separate-Git no-write refusal, ignore-byte variants, source modes, concurrent init, exact-ID continuation and refusal. Its Linux/x86_64 ptrace fixture kills/injects I/O errors at every observed preparation/write/sync/rename/removal boundary; it is not power-loss or all-host evidence. Prepared claim records prove refusal only. Test definitions in `tests/pending/` are not all ignored; inspect each attribute and actual run result.

All template APIs, including pre-init material-only planning/expansion and empty reads, are executed by the runs/templates issue branch under the approved allocation. The base supplies initialization/authority, not simulated templates. The active counterparts below preserve bounded obligations; final connected/installed-package, all-writer cleanup and downstream lifecycle gates remain pending.

## Explicitly superseded assertions

- The beta.1 51-tool inventory and claim/template/storage advertising are
  replaced first by the 13-tool hosting inventory, then the integrated 18-tool initialization/material inventory and now the issue branch's bounded 26-tool run/template inventory. The complete frozen beta.2
  catalog remains required at combined delivery.
- Selecting a linked checkout no longer makes its physical material copies
  authoritative. Active tests check inherited primary routing and explicit,
  read-only physical views; branch selection never changes branches.
- Discovery now includes the actual Git directory and primary checkout.
  Human formatting is not the old two-line beta.1 protocol. Active tests still
  check distinct percent-encoded newline/non-UTF8 paths and literal option values.
- A no-field update is invalid_argument under the shared frozen input contract;
  syntactically unknown/undelivered commands remain usage errors.
- Diagnostic lines are nullable. Active checks require source-linked diagnostics,
  unavailable execution for unsafe/unreadable catalogs, and readiness refusal.
  Directory scan failure must not masquerade as an absent item.
- Concurrency callers explicitly resubmit after detected source/directory capture
  conflicts or storage_busy. This is caller-controlled retry of an unpublished
  one-file operation, not automatic replay of a partial continuation.

## Pending consumers: not passes

Each named `#[ignore = "PENDING ..."]` is a visible, compiled, **unexecuted**
migration obligation with its scheduled owner below. It is not a passing test,
a removed material requirement, owner completion, or a combined beta.2 gate.
Do not enable these unchanged legacy tests as beta.2 acceptance: first migrate
their envelopes/commands and retain applicable behavior and safety assertions.

Ordinary material close/reopen/repair are assigned to initialization.
Ownership-aware forms and acquisition/end/draining are assigned to sessions/
claims; attached close handoffs are assigned to handoffs. Runs/templates and
finalization/disposal retain their scheduled owners. Old global storage
generation recovery, workspace registries/bindings, attach/detach, rebinding,
receiver-retention/pruning and squash/discard command schemas are superseded:
migrate applicable source/ownership/publication safeguards, not those removed
product workflows. The final adoption owner verifies the connected beta.2
workflow. No downstream task is started or closed by this allocation.

### Runs/templates bounded counterparts and remaining compound consumers

The eight run/template operations execute through the real CLI and persistent MCP. `scoped_runs_templates.rs` launches the three new public suites; `scoped_rotation.rs` tests internal read-only guard preparation. `scoped_publication.mjs` injects errors/kills at every observed start/expansion syscall boundary, exercises exact frozen continuation conflicts and pauses real writes to detect outside source changes. This is Linux/x86_64 local evidence, not power-loss/all-host verification. Prepared session/claim/ending records prove guards only.

The retained beta.1 tests below remain compiled and **unexecuted as whole tests**, not passing or silently dropped. Their applicable bounded safeguards are migrated to the named active counterparts. Compound acquisition/completion/end/disposal work remains explicitly pending with its downstream owner; global registration/bindings, attach/detach and perpetual backup-success assertions are superseded, not deferred APIs. None of this closes the current delivery item or proves integration/package acceptance.

| Retained legacy test | Active bounded counterpart / explicitly remaining allocation |
| --- | --- |
| `human_expansion_reports_key_ids_destinations_and_existing_updates` | `scoped_runs_templates.mjs`: human expansion exposes permanent IDs/paths and exact existing updates; pretty JSON replaces the old table formatter |
| `template_preview_matches_cli_and_publishes_nothing` | `scoped_runs_templates.mjs`: v1/v2 symbolic preview, literal parameters, item/default provenance, CLI/MCP equality and complete filesystem no-write fingerprints |
| `template_preview_reports_physical_fallback_warnings_through_cli_and_mcp` | `scoped_run_safety.mjs`: catalog/missing coordination/physical fallback; honest execution/context fields replace beta.1 storage warnings |
| `rootless_planning_and_readonly_mixed_preview_through_cli_mcp` | `scoped_runs_templates.mjs`: pre-init material-only preview/expansion and empty reads; `scoped_run_safety.mjs`: symbolic uninitialized wisps |
| `mixed_expansion_claimed_wisp_completion_and_derived_finish_survive_restart` | `scoped_runs_templates.mjs`: mixed explicit ancestry, close/reopen and derived finish across persistent MCP/CLI; `scoped_run_safety.mjs`: prepared exact-source claim authorization/refusal. Acquisition/claimed completion remains PENDING sessions/claims w-54de7594; attach/detach are superseded |
| `terminal_fixture_keeps_material_binding_and_uncommitted_worktree_state` | `scoped_run_safety.mjs`: selected definitions and whole authoritative source bytes; historical child barrier/material update; `scope_hosting.mjs`: uncommitted host bytes. Workspace binding files are superseded; actual finalization/disposal remains PENDING w-0e159a0e |
| `frozen_existing_wisp_expansion_refuses_before_any_setup_through_cli_and_mcp` | `scoped_run_safety.mjs`: historical existing-wisp rewrite refuses without setup, independent of an explicit run selector. Old squashing/discarding generations are superseded; actual ending transitions remain PENDING w-0e159a0e |
| `frozen_run_root_material_and_wisps_match_ready_vs_acquire_in_cli_and_mcp` | `scoped_runs_templates.mjs`/`scoped_run_safety.mjs`: ended barriers, historical/current wisp gates and independent material CRUD; `scope_hosting.mjs`: prepared ownership diagnostics/readiness. Actual acquisition and ending race remains PENDING w-54de7594/w-0e159a0e |
| `run_start_lookup_errors_and_reference_forms_match_cli_and_mcp_without_invalid_writes` | `scoped_run_safety.mjs`: invalid/missing/ambiguous scope references, all full/prefix forms, unchanged repeat and wisp boundary refusal |
| `membership_validates_all_references_before_setup_with_cli_mcp_lookup_parity` | Attach/detach success is superseded by derived membership. `scoped_runs_templates.mjs`/`scoped_run_safety.mjs`: explicit ancestry, complete graph, selectors and no-write lookup refusal |
| `frozen_target_expansion_is_lifecycle_conflict_while_preview_stays_read_only` | `scoped_run_safety.mjs`: ended target expansion refuses; retained target preview stays symbolic/write-free. Legacy squashing/discarding phases are superseded; pending ending execution remains w-0e159a0e |
| `fresh_template_only_clone_previews_without_writes_and_expands_materials_in_cli_and_mcp` | `scoped_run_safety.mjs`: actual fresh template-only clone, no-write preview and pre-init material expansion with no anchor/run/ignore. `scoped_runs_templates.mjs`: CLI/MCP parity and human CLI expansion. Global registration/bindings are superseded |
| `wisp_update_returns_actual_previous_inode_and_noop_has_no_recovery_in_cli_and_mcp` | `scoped_run_safety.mjs`: actual run-owned source, updated/no-op results and verified obsolete-stage cleanup. Permanent old-version backup success is superseded by DES-FILE-PUBLICATION-LIFECYCLE; stale open handles are a documented limit |
| `expansion_refuses_missing_bound_source_without_recreating_its_catalog` | `scoped_run_safety.mjs`: missing explicit authoritative host catalog refuses without fallback/recreation; whole-file host selection replaces binding files |

### Initialization safety counterparts for retained legacy storage spellings

The still-compiled, ignored beta.1 storage/receipt tests below do not prescribe restoration of receipts, generations or `storage ...` commands. Their applicable safeguards now have real public beta.2 counterparts:

| Retained legacy obligations | Active counterpart |
| --- | --- |
| Fresh reads/idempotence/shared caller initialization; persistent CLI/MCP reloading; private modes | `onboarding.mjs`: read-only status/negotiation; repeated/concurrent primary/linked init; real traced boundaries and restrictive umask |
| Lost-lock expectations, complete/malformed receipt checks, at-most-one reset, completed receipt cannot replace lock | `onboarding.mjs`: lost coordination is unavailable/refuses; malformed progress stays visible; one root from concurrent callers; exact-ID continuation has no permanent receipt |
| Invalid storage CLI/MCP inputs, duplicate selectors, non-UTF8 arguments, unknown operation kinds/prefixed entries | `onboarding.mjs`: argument validation before discovery/preparation, source-linked malformed pending records and no-write fingerprints; active CLI path/option grammar cases |
| Receipt retained archives and prior loss | `onboarding.mjs`: syscall interruption/failure evidence, exact frozen-source conflicts and cleanup; `material_completion.mjs`: malformed/misnamed/shadowed/symlink repair recovery bytes and modes |

Old APIs/generation/receipt success assertions are explicitly superseded, not pending product APIs. Original tests remain compiled and visible for provenance; these counterpart mappings are not claims that ignored tests ran. Lock-contention/ownership-aware behavior and connected all-writer cleanup remain their downstream gates. Do not convert refusal-only checks to simulated downstream successes.

The harness's existing process-helper ignores are separate from this table.
Local hosting verification cannot satisfy release/package/all-host or final
combined acceptance.

| Retained source | Pending test | Scheduled Work owner |
| --- | --- | --- |
| `tests/beta_adoption.rs` | `complete_beta_workflow_and_public_surface_parity` | [w-4334a9b9](../.work/items/4334a9b95506477d880e221673383514.md) |
| `tests/claims_execution.rs` | `claim_acquisition_preserves_cli_and_mcp_lookup_errors_and_valid_references` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `malformed_claims_and_invalid_owner_inputs_create_no_workspace_or_binding` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `not_ready_returns_item_and_evaluated_prerequisite_completed_and_aggregate_blockers` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `batch_list_matches_single_inspection_and_sorts_overlaid_sources_by_full_id` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `batch_list_keeps_ownership_warnings_and_readiness_refuses_malformed_claims` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `close_permission_error_retains_last_ending_publication_and_saved_item_partial` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `unreadable_metadata_preserves_cli_mcp_write_refusal_and_read_warning` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `independent_lock_holder_preserves_cli_mcp_mutation_contention_and_read_warnings` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/claims_execution.rs` | `readiness_reports_global_ownership_conflicts_with_or_without_other_candidates` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/cli_json.rs` | `storage_reads_do_not_initialize_and_warnings_reach_human_output` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/cli_json.rs` | `storage_usage_rejects_invalid_inputs_before_discovery` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/cli_json.rs` | `storage_contention_is_distinct_and_does_not_hide_file_readiness` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/cli_json.rs` | `duplicate_worktree_selectors_refuse_storage_mutations_before_discovery` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/cli_json.rs` | `non_utf8_storage_mutation_arguments_report_not_published_before_discovery` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/context_execution.rs` | `named_rebind_remove_keeps_captured_ownership_and_workspace_across_restart` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/context_execution.rs` | `workspace_reuse_rebind_unbind_and_cleanup_user_evidence` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/context_execution.rs` | `session_scope_strict_inputs_and_corruption_never_become_empty_context` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/context_execution.rs` | `concurrent_set_same_name_has_one_record_and_one_create` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/context_execution.rs` | `wisp_uses_run_default_without_material_binding_and_names_survive_completion` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/context_execution.rs` | `closing_and_missing_workspaces_remain_inspectable_but_refuse_assignments` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/context_execution.rs` | `multiple_missing_current_run_sources_can_be_rebound_one_at_a_time` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `active_material_claim_binding_loss_refuses_divergent_cli_mcp_writes_and_retires_only_after_restore` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `binding_loss_keeps_release_controller_recovery_and_historical_inspection_available` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `workspace_free_core_and_wisp_claims_do_not_require_material_bindings` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `malformed_claim_yaml_uses_real_outer_and_nested_paths_in_cli_and_mcp` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `bound_whole_file_is_read_and_mutated_from_main_without_touching_main_item` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `actual_claim_calls_refuse_uninitialized_and_corrupt_storage_while_reads_warn` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `cli_and_mcp_compete_for_one_owner_then_release_without_item_source` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `checkout_view_survives_missing_bound_source_and_malformed_workspace` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `bound_items_remain_authoritative_when_selected_catalog_is_absent` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `malformed_bound_sources_remain_diagnostics_independent_of_binding_order` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `malformed_context_records_report_the_actual_path_in_cli_and_mcp` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `complete_claim_item_filters_do_not_require_a_live_bound_source` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/coordination.rs` | `bound_header_id_overrides_misnamed_selected_copy_without_hiding_bound_damage` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/finalization.rs` | `squash_keeps_root_exact_routes_output_and_repeats_across_transports` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `material_members_historical_claims_and_existing_digests_survive_later_discard` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `pending_squash_freezes_material_member_edits_but_not_the_root` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `squashing_blocks_outside_completion_changes_creation_templates_repair_and_rebinding` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `symmetric_related_links_block_cleanup_regardless_of_which_endpoint_authored_them` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `other_run_cleanup_cannot_remove_children_of_a_squashing_member` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `unbinding_cannot_expose_unfinished_outside_children_during_squash` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `empty_and_unfinished_runs_are_not_squashable_but_disposable` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `selected_then_full_discard_abandons_work_preserves_material_and_context` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `claims_outside_item_references_and_cross_run_handoffs_block` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `internal_edges_and_abandoned_internal_handoffs_do_not_block` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `mixed_handoff_receivers_allow_cleanup_after_internal_edges_become_dangling` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `truncated_digest_returns_structured_errors_instead_of_panicking` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `linked_surviving_output` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `cross_filesystem_output_when_available` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/finalization.rs` | `strict_adapter_inputs_and_summary_stdin` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/handoffs.rs` | `cli_mcp_restart_deliver_verbatim_many_sources_receivers_and_same_item_context` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `receiver_replacement_preserves_body_and_retention_until_all_cancelled` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `claimed_sources_authorize_all_sources_but_receivers_need_no_pair_and_release_reassign_retain` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `close_with_context_cli_and_mcp_save_then_complete_and_do_not_deduplicate` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `missing_or_invalid_receivers_retain_with_diagnostics_without_stale_checkout_fallback` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `concurrent_receiver_changes_and_pruning_serializes_exact_sets_across_linked_processes` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `cross_run_wisps_deliver_and_retain_after_source_run_finishes` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `derived_aggregate_receivers_resolve_without_changing_graph_edges` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `retarget_to_done_receiver_and_prune_race_never_deletes_an_unresolved_receiver_set` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/handoffs.rs` | `stdin_exact_body_malformed_envelopes_and_storage_recreation_are_visible` | [w-2cc1d6ac](../.work/items/2cc1d6acf22e4a48bbf9839998674264.md) |
| `tests/mcp_protocol.rs` | `storage_mcp_lifecycle_and_warning_payloads_match_cli` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/mcp_protocol.rs` | `storage_mcp_validates_requests_before_selecting_a_checkout` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/selection.rs` | `filters_share_full_graph_exact_labels_and_cli_mcp_ordering` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `all_priority_levels_then_canonical_ids_and_empty_do_not_create_setup` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `reference_errors_and_checkout_run_rejection_match_transports` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `unavailable_or_corrupt_ownership_is_never_empty_dispatch` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `current_run_members_material_wisps_and_root_scope_intersect` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `selected_checkout_and_bound_whole_file_control_all_filters_and_selection` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `independent_cli_mcp_linked_worktree_races_never_repeat_coordination_identity` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/selection.rs` | `source_change_while_waiting_on_checkout_lock_refuses_without_retry_or_claim` | [w-54de7594](../.work/items/54de759473154b008161327e9c961a81.md) |
| `tests/storage.rs` | `fresh_reads_do_not_write_and_initialization_is_shared_and_idempotent` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `lost_lock_inspection_supplies_exact_expectations_to_real_recreation_caller` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_persistent_mcp_and_cli_reload_divergent_selected_views_without_rewriting_entities` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `two_real_callers_with_same_expected_generation_publish_at_most_one_reset` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `completed_receipt_cannot_replace_lost_lock_through_real_cli` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_cli_initialization_and_recreation_use_private_modes_under_restrictive_child_umask` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_cli_and_mcp_require_a_complete_receipt_matching_live_generation_without_writing` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_callers_report_malformed_pending_intent_and_explicitly_retain_it_without_replay` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_cli_and_mcp_retry_complete_receipts_without_rewriting_bytes_or_generation` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_callers_refuse_storage_prefixed_entries_in_operations_without_changing_them` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_callers_validate_matching_receipts_retained_archives_read_only` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/storage.rs` | `real_cli_and_mcp_report_retained_prior_loss_on_initial_and_same_id_results` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/workspace_cleanup.rs` | `closing_failure_retry_external_removal_and_restart_keep_real_context` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `closing_refuses_new_bindings_defaults_claims_and_controller_cleanup` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `cancel_requires_original_path_and_repository_then_allows_reuse` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `current_run_defaults_outputs_roots_and_outside_members_block_cleanup` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `gates_reject_wrong_controller_unexecutable_items_and_missing_sources` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `observed_success_refuses_path_substitutes_and_new_users_without_deleting_context` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `input_validation_and_no_implicit_cleanup` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `independent_process_race_closing_vs_assignment_has_one_safe_outcome` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `closing_blocks_material_creation_updates_repair_and_expansion_without_writes` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `same_context_retry_remains_readable_after_item_closure_or_missing_source` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `nested_controller_or_material_checkout_cannot_be_treated_as_surviving` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `closing_ancestor_refuses_registration_and_claim_setup_in_new_nested_checkout` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `target_containing_repository_shared_storage_cannot_be_closed` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/workspace_cleanup.rs` | `shared_lock_contention_and_return_do_not_hold_lock_during_external_removal` | [w-0e159a0e](../.work/items/0e159a0eb11145d9942f7366262fb4c2.md) |
| `tests/pending/cli_json.rs` | `durable_loop_and_machine_readable_errors` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/cli_json.rs` | `invalid_source_graph_and_ambiguous_prefix_are_distinct` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/mcp_protocol.rs` | `protocol_client_runs_durable_loop_and_matches_cli_results` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/mcp_protocol.rs` | `protocol_schema_rejects_null_wrong_types_and_source_errors` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/mcp_protocol.rs` | `equivalent_cli_and_mcp_mutations_publish_identical_sources` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/claims_execution.rs` | `actual_uninitialized_cli_completion_changes_use_original_durable_operation` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/bootstrap_adoption.rs` | `real_backlog_adopts_and_advances_on_both_surfaces` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
| `tests/pending/bootstrap_adoption.rs` | `roundtrip_restart_and_selected_linked_view` | [w-6bf6198a](../.work/items/6bf6198a34494a94a30a9ba53ed976a6.md) |
