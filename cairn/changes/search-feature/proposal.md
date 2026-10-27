---
cairn: change
id: search-feature
status: active
created: 2026-10-18
---

# The search part, behind a `search` feature

## Why

pimdir SEARCH.md (`draft-04`) specifies the index, its refresh, its extraction and the query language, with 21 query vectors, and no implementation runs them; this crate is the reference implementation of the store and the engine, and has no index. Every consumer pages collections into memory and filters, or calls the store's `LIKE` reads (`search_mail`, `search_contacts`, `search_calendar`, `search_files`), which the spec keeps only "until SEARCH.md's index".

Deferred to the 2027 NLnet plan, Task 2 "pimdir Notmuch", with pimdir's search-milestones change as the plan. This change is milestones M1 to M3 and the CLI half of M4, on this side.

## What

A `search` feature on `client`, off by default, so the Android cdylib and every consumer that does not search carry none of it.

- **The index file**: index.db beside the store, opened as the main database with pimdir.db attached read-only as `store` (SEARCH §2, §3), the vendored migrations/search and queries/search, rebuilt rather than migrated on a version or tokenizer mismatch. The indexer is a store reader, takes `index.lock` exclusively for a refresh, and is the only writer of index.db.
- **The refresh** (SEARCH §4): the change feed folded in bounded read transactions, a body indexed once per hash, placements, summary text, flags, occurrences and threads upserted per row, the key reconciliation when `store_meta.purges` or a collection stamp moved, the cursor written in the transaction completing the pass.
- **Extraction** (SEARCH §6): mail through a MIME walk with transfer encodings, charsets, RFC 2047 and 2231, `text/plain` preferred and `text/html` reduced to text; vCard and iCalendar from their parsers; `encrypted` and `unparseable` statuses; the summary fallback for a bodiless placement. The walks keep the depth bound summaries already have (`MIME_DEPTH_MAX`).
- **Occurrences** (SEARCH §7): the horizon, expansion through ical-rs's recurrence set, the roll through `items_to_reexpand`, a bound on what one item writes.
- **The query**: a parser to an AST (exposed, so himalaya compiles its own filter into it), a compiler to FTS5 `MATCH` plus SQL, hits as `(account, seq)` with placements, the union rule for tags, coverage in every answer.
- **The CLI**: `index`, `search`, `count`, `show`, `thread`, `tag` (a producer's `set-flags` per placement, the source resolved as the queue verbs resolve it). cli.md's "SHALL NOT interpret item content" becomes "never renders it": extraction is the spec's rule, not the tool's opinion.
- **Conformance**: tests/search.rs runs every case of vectors/search, skipping when the spec checkout is absent, as tests/summaries.rs does.

## Coverage, three axes

Borrowed from an external engine's search: coverage as three axes rolled up conservatively, so a client can say "older messages not downloaded" or "searching the server" from one value instead of guessing. SEARCH §8.2 already makes coverage part of every answer (bodiless items per kind, the range leaving the horizon, body or summary per hit); what is missing is one type a client reads and a rule for combining it.

- **Local**: items in scope with no body (unindexed content), and items whose body is held but not indexed yet (the refresh behind the feed).
- **Temporal**: `Full` when the query's range lies inside every queried collection's coverage (`covered_since`, `covered_until`, STORAGE §10), else `Bounded` by the date below which the store is not known to hold every member, or a round still filling.
- **Remote**: `LocalOnly`, or `Augmented { exhaustive }` when a consumer merged a server-side search into the hits. The library holds no credentials and never searches a server; the axis is there so a consumer that does reports it in the same value.

The roll-up over collections and kinds keeps the worst of each axis, and `is_complete()` is true only when nothing is bodiless or unindexed in scope, the range is `Full`, and the remote axis is `LocalOnly` with nothing to add or `Augmented { exhaustive: true }`. The same type serves the store's `LIKE` reads before the index exists: their temporal axis is already answerable from the coverage rows.

## Decided (2026-10-27)

Task 2 prepares Task 1 "Pimalaya CLI: REPL and MCP"; the decisions shared with the spec are in pimdir cairn/changes/search-milestones. On this side:

- **Parsers**: mail-parser 0.11 with `full_encoding` (as himalaya, neverest and the GTK app use it), ical and vcard, as optional dependencies of `search`. The summary scanners (src/summary) stay as they are for the store. Extraction records chunk boundaries per object hash.
- **FTS5**: through `vendored`, the default; without it, opening the index fails with a clear error when the system SQLite lacks FTS5.
- **The `LIKE` reads**: kept for consumers built without the feature, and they fill the same coverage type.
- **Who refreshes**: a one-shot verb refreshes up to the cursor before it answers, within a time bound, and coverage reports what is still behind; a session (REPL, MCP) follows the feed. neverest does not index.
- **The query**: an unknown field is refused with a structured error (position, expected, known fields). Every search verb has `--json` and a JSON Schema from M1; a hit carries a snippet and an address `show`, `thread` and `tag` accept as is.
