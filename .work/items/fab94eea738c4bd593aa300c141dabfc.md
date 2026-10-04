---
format_version: 1
id: "fab94eea738c4bd593aa300c141dabfc"
title: "Implement stable-identity wisp promotion"
completion: manual
state: open
priority: 2
parent: "16e2d0f58f0b4386b6d042bf1c8380b8"
depends_on: ["54de759473154b008161327e9c961a81"]
labels: ["beta.2", "implementation"]
---
## Scope

Promote an unclaimed temporary item into scope-resolved material storage without replacing its identity, body, state or relations. Keep partial cross-location publication inspectable.

## Acceptance criteria

- Current claims block promotion; candidate source/destination and graph are validated before publication.
- Full ID, body bytes, recorded completion, ancestry and relation identity are unchanged; only location/persistence changes.
- Execution commits alone do not promote items; a real Git merge retains the resulting material file in its surviving source.
- Interrupted moves cannot silently lose the source or make duplicate copies independently claimable; progress/conflicts are exposed for narrow continuation.
- Real CLI/MCP and fault tests plus applicable code/Mara checks establish evidence.

## Mara contracts

`REQ-WISP-PROMOTION`, `DES-SCOPE-TRANSITIONS`, `VER-PROMOTION-DISPOSAL`. Read the accepted beta.2 contracts and freeze any still-draft wire details before behavior changes.

## Delivery

Start from current main on feature/w-<short-id>-<description>; one bounded PR. Preserve unrelated state. State-changing tests use session-owned disposable fixtures. Report actual verification and limitations; keep open until acceptance passes and the change is merged. This item is not publication or real-resource deletion authority.