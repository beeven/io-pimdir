---
cairn: log
change: search-feature
date: 2026-10-27
---

# Threads and relative dates, milestone M3

The last of SEARCH.md's query language the index did not answer: pimdir de34a38's 115 search cases now all run, none skipped.

## What landed

- client::search::thread: `relink` during the refresh for the messages a batch touched (a new, rebodied, rekeyed or dropped placement), parents from the held body's headers through mail-parser or from `mail_summary.in_reply_to`; `rethread` recomputes each connected thread, drops unnamed phantom rows, writes the new thread row and repoints before deleting the unused one.
- `thread:` through the canonical `thread_of`, accepting any message id of the thread. `PimdirIndex::threads` and CLI `search --threads` (`pimdir-search-threads`).
- Relative dates: the AST's `PimdirQueryDate::{Today, Yesterday, Relative { amount, unit }}` and `PimdirQueryDateUnit`, resolved in UTC against `PimdirSearchClock`, which `PimdirSearchOptions::clock` and `count` now take.
- The compiler can no longer refuse: `PimdirQueryErrorCode::UnsupportedField` is removed.
- An own `index_build` table marks the build; an index without it, or with another build, is rebuilt on open, so an index from M2 gains its threads.

## Verification

- `cargo test` default 576, `--features search` 599, all passing; clippy clean on the three feature sets. `--all-features` fails only tests/spec_drift.rs, because the pimdir checkout holds uncommitted storage work (task summaries, `link_parents_*`, `link_sources_*`) another session is doing; the search suites pass.
- tests/search.rs: every vector case, `threads` cases compared as sets of (thread, count, first, last, hits); five thread tests (two replies to a parent never held, a late parent joining and renaming, a join leaving one row with the old id still resolving, a cycle, a deleted member), each shown to fail with its code broken; relative spans (`1y` from 29 February, `3d..1d`, `yesterday..today`, `..2w`, the year-0 clamp).
- End to end on the Stalwart store of M1 and M2: the M2 index rebuilt on open, `thread:reply-1@pimalaya.org` answered both messages, the listing grouped them with count 2 and both dates, and relative dates answered.

## Not done

- M4 (`tag`, the `thread` verb).
- The listing looks up each hit's thread with one query per hit before paging: unmeasured at volume. A cycle edge ignored once is not reconsidered when the other edge goes. `2w..2w` reads as `2w`. Spans are UTC only.

## Capabilities moved

- search: "Threads are relinked by the refresh and listed on request" and "Relative dates resolve against a clock" (added); the rebuild, query, hits and vectors requirements (modified).
- cli: the verb surface (modified: `search --threads`).
