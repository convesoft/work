:::mara design DES-CLAIM-API
:mid: 01M3TWR5NB3XQV4AK3DHK5BFJJ
:title: Emit immutable claim acquisitions and explicit endings
:status: accepted
:kind: interface
:satisfies: REQ-CLAIM-EXCLUSION
:satisfies: REQ-CLAIM-RECOVERY
:satisfies: REQ-WORK-SELECTION

## File envelopes and current ownership

One acquisition is `claims/<claim-id>.yaml`, with exact fields `format_version, store_id, recovery_generation, id, item_id, actor, session, acquired_at` and optional `workspace_id, run_id, session_record_id`. These are immutable acquisition facts. Session has the shape in [[DES-EXECUTION-IO]]. Workspace/run references record the context at acquisition; they do not override a later explicit material binding. A new owner or reacquisition always receives a new claim ID, including the same session returning.

End an acquisition by create-new `claims/<claim-id>.end.yaml`, with exact fields `format_version, store_id, recovery_generation, claim_id, item_id, ended_at, actor, reason, outcome`, where `outcome` is `released|completed|reassigned`, and optional `recovery: true`. Reason is opaque text and may be empty for an ordinary release/completion; controller recovery requires a nonempty reason. An optional session-record reference is context only; its captured external session must match at acquisition and later rebinding cannot transfer ownership. Historical claims may reference removed wisps, terminal runs or deleted named-session bindings; those audit references do not block cleanup. A matching end file makes the acquisition historical; no claim is overwritten, deleted, or assigned to another owner. This single terminal fact is not a generic event log.

The current owner is the sole valid acquisition for an item in the current store generation without a valid matching end file. More than one is `claim_conflict` with all IDs; malformed acquisition/end, dangling end, mismatched identity or unsupported format refuses ownership mutation with diagnostics. Never pick the newest timestamp. Retained old-generation files cannot authorize anything. Foundation recreation archives old live entities; normal claim operations do not import that archive. No automatic claim pruning, timeout, heartbeat or notification service.

## Operations and CLI/MCP mapping

All mutations reload claims under an exclusive `CoordinationGuard`. Acquire/reassign also require a valid resolved graph and eligible candidate; release/recover remain available when the claimed item's source is missing or its graph is invalid, because ending ownership does not require modifying that item. `ClaimStore` owns claim parsing, lookup, creation/end IO and authorization; the coordinator owns resolved graph/worktree loading and item mutation. The worker takes a guard plus a validated candidate snapshot; adapters never construct readiness booleans from user input.

| CLI | MCP | Arguments beyond selected worktree | Result |
| --- | --- | --- | --- |
| `claim acquire ITEM --actor A --session-namespace N --session-id S` | `claim_acquire` | `item, actor, session` | `{claim, item, changed:true}` |
| `claim inspect CLAIM_ID` | `claim_inspect` | `claim_id` | `{claim, ending, current}` |
| `claim list [--item ITEM] [--current]` | `claim_list` | optional `item`, `current_only` default false | `{claims:[{claim,ending,current}]}` |
| `claim release CLAIM_ID --session-namespace N --session-id S [--reason TEXT]` | `claim_release` | `claim_id, session, reason?` | `{claim, ending, changed}` |
| `claim recover CLAIM_ID --actor A --reason TEXT --executors-stopped` | `claim_recover` | `claim_id, actor, reason, executors_stopped:true` | `{claim, ending, changed}` |
| `claim reassign CLAIM_ID --actor A --session-namespace N --session-id S --reason TEXT --executors-stopped` | `claim_reassign` | `claim_id, actor, session, reason, executors_stopped:true` | `{previous_claim_id, claim, item, changed:true}` |

Acquire requires an executable open manual item with satisfied prerequisites and no current owner. Aggregates/done/blocked items return `not_ready` and blockers. Existing ownership returns `claim_conflict`, even for the same session: a retry reads the current claim rather than minting another one. The request does not choose a claim ID. An item can be claimed without a run; context fields are inferred from current view, not alternate exclusion namespaces.

Ordinary release matches the acquisition's session. Repeating release of the same already-released claim returns its ending with `changed:false`; a different ending or wrong session returns `stale_claim`. Completion is coordinated with ordinary item close: validate authorization, save optional outgoing handoff(s), persist item close, then emit the completed ending. If interrupted after close, the claim remains current and blocks reassignment until the same owner retries close/release or controller recovers it. Retrying close of an already-done item with the still-current pair may finish its ending without rewriting its body/state. Do not release first and then close. The coordinator owns this ordering and adapter wiring.

Recover ends abandoned ownership without changing item completion or inventing a replacement. Reassign validates the old current claim and new eligible candidate first, then emits the old ending and the new acquisition under one lock. A crash between the two can leave no current owner; report partial progress. A retry of the old reassign is stale, and the controller inspects then acquires if appropriate. No general transaction engine or replacement claim is silently reconstructed. Both recovery verbs require caller assertion that affected executors are stopped; this is not remote process control.

## Scoped selection, delivered by the dependent selection item

List/ready/claim-next filters are optional `root` (root plus transitive children), `run_id` (current run members, excluding its root unless explicitly a member), `labels_all` (all exact labels), `priority_max` (0–4) and `persistence: material|wisp`. Combine filters with AND; no inherited labels, model matching or arbitrary expressions. CLI uses `--root, --run, --label` repeated, `--priority-max, --persistence`. Empty scope yields an empty list.

`claim next` / `claim_next` takes the same filters plus actor/session; inside the one lock reload graph/claims, sort eligible unowned candidates by priority then full ID, and publish at most one acquisition. Return `{claim:null,item:null,changed:false}` when none are eligible. Listing never reserves. Baseline claim acquisition does not wait for claim-next delivery.

## Meaningful acceptance

Independent CLI/MCP processes racing one material item yield one owner; different items can both succeed. Same-session reacquisition has a new ID and stale pairs fail. Verify append-only acquisition bytes, ending retries, completed-item/end interruption, explicit reassignment gap, unchanged ownership across restart, storage-unavailable refusal, malformed ending refusal and source-conflict refusal. Notifications and wall-clock advancement must not affect exclusion. The later runs integration repeats ownership checks for wisps without changing claim format.
:::
