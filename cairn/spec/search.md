---
cairn: spec
capability: search
status: current
---

# Search

The reference implementation of pimdir SEARCH.md (`draft-04`): index.db beside the store, its refresh from the change feed, extraction and the query language. It ships behind the `search` feature on `client`, which `cli` enables and the default leaves out, so a consumer that does not search (the Android cdylib) carries none of it. Milestones M1 to M4 of pimdir's search-milestones: every kind's extraction, every field, occurrences within the horizon, threads, relative dates, tagging and the thread verb.

### Requirement: The index is derived and rebuilt, never migrated
`PimdirIndex::open` SHALL open index.db as the main database with pimdir.db attached read-only as `store`, and SHALL rebuild it when its version or its tokenizer differs from the vendored schema's, or when its own `index_build` marker is absent or differs from `sql::search::BUILD`, so an index built by an earlier release of this crate is rebuilt rather than answering from data it lacks. It SHALL refuse a SQLite without FTS5 or older than 3.43 (`PimdirError::Fts5`) rather than build a partial index.

### Requirement: A refresh folds the feed in bounded, resumable batches
`PimdirIndex::refresh` SHALL take index.lock without waiting, answering `busy` when another indexer holds it. It SHALL fold `load_changed_items` in transactions of a bounded number of rows, each full batch recording its last row's stamp with the previous pass's purge count, so a pass cut short resumes where it stopped. The transaction exhausting the feed SHALL reconcile keys when the purge count or a collection stamp moved, and record the cursor and purge count read before the pass. A deadline SHALL be checked between batches.

### Requirement: One-shot answers refresh first and say what is behind
`search` and `count` SHALL refresh up to the store's head within `--refresh-timeout` before answering (`0s` skips it), and SHALL count what stays unfolded as `coverage.local.unindexed`. A session that keeps the index open follows the feed instead.

### Requirement: Mail is extracted by mail-parser, chunked as SEARCH §6 says
Mail extraction SHALL go through mail-parser (`full_encoding`): the MIME walk under `MIME_DEPTH_MAX`, transfer encodings, charsets, RFC 2047 and 2231, `text/plain` preferred and `text/html` reduced to text, `encrypted` and `unparseable` statuses, the attachment field from the crate's own stand-in walk. The `body` and `note` fields SHALL be cut into chunks by `search::chunk::chunks` on SEARCH §6's rule. Contacts and calendar items SHALL be extracted through vcard-rs and ical-rs on SEARCH §6's table, a `TEL` as written and as its digits, every event, task and journal of a resource, overrides included; an item falls back to its summary only while bodiless or unparseable, and a file always reads its summary.

### Requirement: Calendar time is expanded through ical-rs within a rolling horizon
Occurrences SHALL be expanded by ical-rs's `IcalRecurTimeline` with zones from `ical::tzdb::zones`, so the resource's own `VTIMEZONE` wins over the bundled database and an unknown `TZID` reads floating, within a horizon of one year back and two forward around the refresh's `now`, at most `OCCURRENCES_MAX` per item. An item SHALL be re-expanded when its body changes, when the horizon has rolled by a day or more, and when `ical::tzdb::version()` differs from the recorded one, which `set_tzdb_version` then records. A resource holding overrides and no master yields its overrides. `date:` and `when:` SHALL compare a bodied calendar item's occurrences, and coverage SHALL report `horizonLeft` when a range leaves the horizon with a calendar collection in scope.

### Requirement: A query is parsed whole or refused with SEARCH §8.4's object
`PimdirQuery::parse` SHALL accept exactly SEARCH §8.3's grammar and refuse anything else with a `PimdirQueryError` carrying §8.4's code and span, never reading an unknown field as text. On `unknown-field` its `expected` SHALL be empty, `fields` carrying the list, and a name that lowercases to a field SHALL be named in the message. A statement failing on a query the grammar accepted SHALL answer `PimdirError::Unanswerable`, never a raw SQL error. The AST SHALL be public, so a consumer can compile its own filter into it.

### Requirement: Every answer carries its coverage
A search answer SHALL carry a `PimdirSearchCoverage`: the local axis (bodiless mail, contacts and calendar items in scope, never files, and the items not yet folded), the temporal axis (`Full`, or `Bounded` by the collections' coverage rows), and the remote axis (`LocalOnly` unless a consumer merged a server answer). The roll-up keeps the worst of each axis. `PimdirReader::search_coverage` SHALL fill the same type for the store's `LIKE` reads, which stay for consumers built without the feature.

### Requirement: Hits are addressable and typed
A hit SHALL be `(account, seq)` with its placements, its kind and title, what it matched (`body`, `summary`, or none when only structured predicates matched) and a snippet cut from its chunks. The CLI's `--json` output SHALL use camelCase keys and publish its JSON Schema (`pimdir-index`, `pimdir-search`, `pimdir-count`, `pimdir-show`, `pimdir-search-threads`, `pimdir-thread`, `pimdir-tag`, `pimdir-query-error`), and `show SEQ --account ACCOUNT` SHALL accept a hit's address as is.

### Requirement: The vectors are the bar
tests/search.rs SHALL run every case of pimdir vectors/search (and vectors/search/chunks.json) against a store built as an owner leaves it, skipping when the spec checkout is absent. Every case SHALL run: none is skipped, `threads` cases included.

### Requirement: Threads are relinked by the refresh and listed on request
The refresh SHALL relink the messages a batch touched (SEARCH §9): the parent from the held body's first `In-Reply-To` id, else its last `References` id, or from `mail_summary.in_reply_to` for a bodiless item; a `message` row for every parent named, present or not; a cycle broken by ignoring the edge that would close it; the thread id the present member with the lowest `(sort_key, link_id)`, its new row written and members repointed before an unused row goes. `thread:` SHALL accept the thread id or any message id in it, so an id renamed by a join still resolves. `PimdirIndex::threads` (CLI `search --threads`) SHALL group hits by account and thread, in the order of each thread's first hit, `count`, `first` and `last` covering the thread's live members in that account; an item that is not mail stands alone.

### Requirement: Relative dates resolve against a clock
`today`, `yesterday` and relative amounts SHALL resolve in UTC against `PimdirSearchClock`, the system clock unless one is given, as SEARCH §8.1 states: a relative amount alone spans from its instant to now, a relative point as a range side is its instant, and a span reaching before year 0 is clamped to it.

### Requirement: Tagging stages through the queue, each placement as its own source
`PimdirIndex::tag` SHALL plan a flag change over a query's hits without writing: per placement its next flag set, starting from the last pending `set-flags` of that placement so tags staged in a row compose, `read`, `flagged`, `answered` and `draft` naming their system flags, `unread` and `encrypted` refused. Its source SHALL be resolved per placement from the sources syncing its collection: the requested one when it syncs it, else the only one; several and none requested is `ambiguousSource`, a requested one not syncing it is `unsynced`, and a mail source whose declaration lacks a capability the change needs is `unsupported`, naming what is `missing`; none of these is staged, and the other placements are. `stage` SHALL stage one `set-flags` per placement that changes and resolves, after every action passes the producer's check, so a refusal leaves the queue untouched. Each placement SHALL say whether its source can carry the change by its declared capabilities (`syncs`): a tag on a contact or calendar placement, or on mail whose source declares no matching capability, is kept in the store only. The CLI `tag` SHALL drain when the owner role is free and read each outcome back from the store (`applied`, `queued`, `unchanged`, `refused`, ...); `--dry-run` writes nothing.

### Requirement: An item's thread is answered from the index
`PimdirIndex::thread(account, seq)` SHALL answer the item's thread through `thread_of` and `thread_members`, members oldest first, with its id, count, first and last; an item that is not mail, or not threaded, answers itself alone; none when no live placement of it is in the account.
