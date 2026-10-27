---
cairn: spec
capability: search
status: current
---

# Search

The reference implementation of pimdir SEARCH.md (`draft-04`): index.db beside the store, its refresh from the change feed, extraction and the query language. It ships behind the `search` feature on `client`, which `cli` enables and the default leaves out, so a consumer that does not search (the Android cdylib) carries none of it. Milestones M1 and M2 of pimdir's search-milestones: every kind's extraction, every field but `thread:`, occurrences within the horizon; threads and relative dates are M3.

### Requirement: The index is derived and rebuilt, never migrated
`PimdirIndex::open` SHALL open index.db as the main database with pimdir.db attached read-only as `store`, and SHALL rebuild it when its version or its tokenizer differs from the vendored schema's. It SHALL refuse a SQLite without FTS5 or older than 3.43 (`PimdirError::Fts5`) rather than build a partial index.

### Requirement: A refresh folds the feed in bounded, resumable batches
`PimdirIndex::refresh` SHALL take index.lock without waiting, answering `busy` when another indexer holds it. It SHALL fold `load_changed_items` in transactions of a bounded number of rows, each full batch recording its last row's stamp with the previous pass's purge count, so a pass cut short resumes where it stopped. The transaction exhausting the feed SHALL reconcile keys when the purge count or a collection stamp moved, and record the cursor and purge count read before the pass. A deadline SHALL be checked between batches.

### Requirement: One-shot answers refresh first and say what is behind
`search` and `count` SHALL refresh up to the store's head within `--refresh-timeout` before answering (`0s` skips it), and SHALL count what stays unfolded as `coverage.local.unindexed`. A session that keeps the index open follows the feed instead.

### Requirement: Mail is extracted by mail-parser, chunked as SEARCH §6 says
Mail extraction SHALL go through mail-parser (`full_encoding`): the MIME walk under `MIME_DEPTH_MAX`, transfer encodings, charsets, RFC 2047 and 2231, `text/plain` preferred and `text/html` reduced to text, `encrypted` and `unparseable` statuses, the attachment field from the crate's own stand-in walk. The `body` and `note` fields SHALL be cut into chunks by `search::chunk::chunks` on SEARCH §6's rule. Contacts and calendar items SHALL be extracted through vcard-rs and ical-rs on SEARCH §6's table, a `TEL` as written and as its digits, every event, task and journal of a resource, overrides included; an item falls back to its summary only while bodiless or unparseable, and a file always reads its summary.

### Requirement: Calendar time is expanded through ical-rs within a rolling horizon
Occurrences SHALL be expanded by ical-rs's `IcalRecurTimeline` with zones from `ical::tzdb::zones`, so the resource's own `VTIMEZONE` wins over the bundled database and an unknown `TZID` reads floating, within a horizon of one year back and two forward around the refresh's `now`, at most `OCCURRENCES_MAX` per item. An item SHALL be re-expanded when its body changes, when the horizon has rolled by a day or more, and when `ical::tzdb::version()` differs from the recorded one, which `set_tzdb_version` then records. A resource holding overrides and no master yields its overrides. `date:` and `when:` SHALL compare a bodied calendar item's occurrences, and coverage SHALL report `horizonLeft` when a range leaves the horizon with a calendar collection in scope.

### Requirement: A query is parsed whole or refused with SEARCH §8.4's object
`PimdirQuery::parse` SHALL accept exactly SEARCH §8.3's grammar and refuse anything else with a `PimdirQueryError` carrying §8.4's code and span, never reading an unknown field as text. On `unknown-field` its `expected` SHALL be empty, `fields` carrying the list, and a name that lowercases to a field SHALL be named in the message. A field the compiler does not answer yet SHALL be refused as `unsupported-field`, this crate's own code, never answered as a silent miss. A statement failing on a query the grammar accepted SHALL answer `PimdirError::Unanswerable`, never a raw SQL error. The AST SHALL be public, so a consumer can compile its own filter into it.

### Requirement: Every answer carries its coverage
A search answer SHALL carry a `PimdirSearchCoverage`: the local axis (bodiless mail, contacts and calendar items in scope, never files, and the items not yet folded), the temporal axis (`Full`, or `Bounded` by the collections' coverage rows), and the remote axis (`LocalOnly` unless a consumer merged a server answer). The roll-up keeps the worst of each axis. `PimdirReader::search_coverage` SHALL fill the same type for the store's `LIKE` reads, which stay for consumers built without the feature.

### Requirement: Hits are addressable and typed
A hit SHALL be `(account, seq)` with its placements, its kind and title, what it matched (`body`, `summary`, or none when only structured predicates matched) and a snippet cut from its chunks. The CLI's `--json` output SHALL use camelCase keys and publish its JSON Schema (`pimdir-index`, `pimdir-search`, `pimdir-count`, `pimdir-show`, `pimdir-query-error`), and `show SEQ --account ACCOUNT` SHALL accept a hit's address as is.

### Requirement: The vectors are the bar
tests/search.rs SHALL run every case of pimdir vectors/search (and vectors/search/chunks.json) against a store built as an owner leaves it, skipping when the spec checkout is absent. A case this milestone does not answer SHALL be listed with its reason, and one naming a field SHALL be checked to be refused as `unsupported-field`.
