---
cairn: tasks
change: deep-mime-regression
---

- [ ] tests/deep_nesting.rs: a generator for the crash input's shape (8000 levels, boundaries `b{level}` with no suffix so they prefix-collide, `From`/`To` and no `Message-ID`, a `text/plain` leaf `hi`), run on a 1 MiB thread, asserting `(Some(false), None)` from `mail::derive`
- [ ] Check it fails on the parent of the-walk-of-a-body-is-bounded's fix (the recursive walk) and passes now
- [ ] Log entry (no capability moves); no CHANGELOG line, the fix already has one
