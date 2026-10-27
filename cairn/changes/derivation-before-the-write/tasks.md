---
cairn: tasks
change: derivation-before-the-write
---

- [ ] src/client/producer.rs: derive `add` and `update` summaries before `apply_queued` opens its transaction; re-check kind and row inside
- [ ] src/client/write.rs and src/client/attachments.rs: the walk's result computed before the batch and passed in; the write only records it
- [ ] Test: a drain and a body write whose transactions read no blob (a blob store that fails reads inside a transaction)
- [ ] Bench: writer-lock hold time for a 25 MB message before and after
- [ ] Spec: store.md; fold, log, CHANGELOG
