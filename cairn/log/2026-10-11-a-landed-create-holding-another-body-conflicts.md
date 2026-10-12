---
cairn: log
change: a-landed-create-holding-another-body-conflicts
date: 2026-10-11
---

# A landed create holding another body conflicts

Found through neverest's audit baseline: two CardDAV endpoints holding one `UID` under two bodies before their first sync ended with the source's body on both, the target's silently overwritten. The hub projects the source's item onto the target as a pending create, the target's page lands it (SYNC §6), and `PimdirSync::land` took the arrival's body as the base while keeping the staged one, so the staged body was pushed as a local edit. Present since the page names its members (`35a1c3f`, 0.7.0).

## What landed

- src/sync.rs `land`: a mutable arrival carrying a body other than the staged one keeps the staged body and the arrival's flags over a base holding no body and no revision, so the content axis reads both sides changed and follows the policy (`Manual` parks the arrival's body as the divergence, `PreferLocal` pushes gated on its revision, `PreferRemote` pulls). Mail and any arrival with the staged body land as before.
- tests/engine/hub.rs: a landed create holding another body conflicts rather than overwrites (fails before: one push), and follows `PreferLocal`.
- tests/engine/property.rs: the mutable model resolves conflicts parked in its archive, on the server's body. A move relocating a server edit lands a create holding the old body, which the engine used to push over the edit; it parks now.
- Capabilities moved: sync ("A create waits for the page that lands it", modified).

## Not done

`PimdirUpgrade::land` keeps the old shape: a mutable create reaches it only through a listing carrying no body, and it holds no policy. Same overwrite there, unreached by any connector listing bodies.

## Verification

`cargo test --all-features`, `cargo test --no-default-features --features cli --test engine` and clippy `-D warnings`, run on a copy of the tree with src/summary/{mail,calendar}.rs at `HEAD` (another change was editing them). neverest's tests/endpoints.rs green on Radicale against `be03184` with this patch.
