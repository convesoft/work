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

Selected mode: bounded local beta.2 scope. Manual owner: `3932a6c6ff404fb0b7cdf0ace756c2a2`. Declared integration target: `feature/w-3932a6c6-beta2-scope`; this included item does not require its own PR.

Start from the current clean scope tip on `feature/w-<short-id>-<description>`. Preserve unrelated state. Implement, verify and complete required local review before squash integration under docs/delivery.mara.md. Record the actual scope base, reviewed/tested revisions and integrated commit; verify the integrated result and close only after acceptance and local review pass. The owner retains the final combined, cloud-review/CI and remote-main gates. Do not treat old test SHAs as evidence for rewritten commits.

State-changing checks use session-owned disposable fixtures. No push, history rewrite, publication or real-user-resource deletion is authorized by this item.