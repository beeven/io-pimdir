---
cairn: tasks
change: an-ambiguous-intent-awaits-confirmation
---

- [ ] pimdir: STORAGE §15.2 (a drain skips an unconfirmed row), §15.4, §15.5 (the state, confirming, retrying, withdrawing), Annex B.2 (which intents can end so), the statements, a scenario in checks/invariants.sh
- [ ] src/client: mark a row unconfirmed, confirm it (acknowledge with its receipt), retry it, read the state
- [ ] src/intent.rs: the state in the intent vocabulary
- [ ] CLI: the queue verbs show and resolve an unconfirmed row
- [ ] Test: a submit ending unconfirmed is not drained again until confirmed or retried
- [ ] Spec: store.md; fold, log, CHANGELOG
- [ ] neverest: report a submission lost after the data as unconfirmed rather than failed
