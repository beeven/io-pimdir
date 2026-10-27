---
cairn: tasks
change: search-feature
---

# Tasks

Deferred to the 2027 NLnet plan, Task 2 "pimdir Notmuch"; the plan is pimdir cairn/changes/search-milestones.

## M1, the index

- [x] `search` feature on `client`, with mail-parser 0.11 (`full_encoding`, as himalaya) as its optional dependency; vendored migrations/search and queries/search, tests/spec_drift.rs extended (ical and vcard join with M2's extraction)
- [x] FTS5 through `vendored`; a clear error when the system SQLite lacks it
- [x] Open index.db with pimdir.db attached read-only; rebuild on a version or tokenizer mismatch; `index.lock`
- [x] Refresh from the change feed, bounded batches, key reconciliation, cursor in the completing transaction
- [x] One-shot verbs refresh up to the cursor first, within a time bound, staleness reported in coverage; a session (REPL, MCP) follows the feed by calling `refresh` (the session itself is Task 1's)
- [x] Mail extraction through mail-parser, with chunk boundaries; `encrypted`, `unparseable`; summary text for bodiless placements
- [x] Query parser, exposed AST, compiler; an unknown field refused with a structured error; hits with placements, a snippet and an address, coverage
- [x] CLI `index`, `search`, `count`, `show`; cli.md amended from never-interprets to never-renders
- [x] `--json` for these verbs and their JSON Schema (hits, coverage, errors)

## M2, cross-domain

- [ ] vCard and iCalendar extraction; the role fields
- [ ] `kind:`, `with:` (answered since M1, `with:` from the summaries' address rows), `person:`, `linked:`
- [ ] Occurrences within the horizon; the roll; the per-item bound

## M3, threads and dates

- [ ] `message` and `thread` tables, `thread:`, threaded listing
- [ ] Relative dates (`today`, `2w`, ranges)

## M4, this crate's part

- [ ] CLI `tag` through the queue; `thread`; their `--json` and JSON Schema

## Coverage

- [x] A `PimdirSearchCoverage` with the local, temporal and remote axes, the conservative roll-up and `is_complete()`
- [x] Filled by every search answer and by the store's `LIKE` reads (through `PimdirReader::search_coverage`) (temporal axis from the coverage rows); those reads are kept for consumers without the feature
- [ ] pimdir: SEARCH §8.2 states the temporal axis beside the bodiless count; a vector with a range below a collection's coverage
- [ ] Consumers: himalaya and android cairn/changes/a-search-says-what-it-covered

## Conformance and landing

- [x] tests/search.rs over vectors/search (M1 cases; M2 and M3 cases in an explicit skip list)
- [x] Spec: a search capability file (cairn/spec/search.md); cli.md modified
- [ ] Fold, log, CHANGELOG (M1 logged 2026-10-27-search-index-m1; again at each milestone)
