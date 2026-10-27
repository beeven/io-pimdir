---
cairn: tasks
change: raw-payloads-print-their-length
---

- [ ] Choose: a `PimdirBytes` newtype (breaking) or hand-written `Debug` on the carrying types
- [ ] src/remote.rs, src/change.rs, src/mutate.rs and every other public type holding a body
- [ ] Test: no body byte in any of their `Debug` output; a trace-level sync log of a known body does not contain it
- [ ] Spec: seam.md; fold, log, CHANGELOG
