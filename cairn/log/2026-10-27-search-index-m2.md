---
cairn: log
change: search-feature
date: 2026-10-27
---

# The search index crosses domains, milestone M2

Contacts and calendars join mail in the index: pimdir SEARCH.md as of de34a38 (temporal coverage, override-only series, a case per field), on ical-rs 99b9273 (occurrences as instants, a versioned time zone database).

## What landed

- `search` enables ical-rs (`parser`, `tzdb`) and vcard-rs (`parser`). ical-rs is pinned by git to 99b9273 until its next release.
- `client::search::extract`: `PimdirExtract::contact` and `PimdirExtract::calendar` on SEARCH §6's table, `TEL` digits, every component of a resource; `PimdirExtract::body` dispatches on the collection kind, snippets too.
- `client::search::occurrence`: the horizon around `PimdirIndexRefreshOptions::now`, expansion through `IcalRecurTimeline` and `ical::tzdb::zones`, `OCCURRENCES_MAX` per item, re-expansion on a body change, a horizon roll and a database version change; `PimdirIndexReport` gains `expanded` and `bounded`.
- `person:` and `linked:` (own statement `LOAD_ENDS`); `date:`/`when:` over occurrences; `PimdirSearchCoverage::horizon_left`, printed `horizonLeft`.
- `PimdirError::Unanswerable`: a statement failing on an accepted query is typed, never a raw SQL error.

## Verification

- `cargo test` default 576, `--features search` 593, `--all-features` 594, all passing; clippy clean on the three.
- tests/search.rs over pimdir's 115 search cases: every case but M3's answered, `temporal` and `horizon_left` compared on every case. `every_field_alone_answers_in_every_order` runs each text, role and person field, a prefix, a phrase and a negation in the three orders.
- End to end on local Radicale (CalDAV, CardDAV) and Stalwart (IMAP) synced by neverest: `person:alice` answered her card, the mail she sent or was copied on and the meeting she organises; a weekly `Europe/Paris` series with no `VTIMEZONE` expanded through the database at 08:30Z in winter and 07:30Z after 2026-03-29; the override-only meeting, a `DUE`-only task and an all-day event answered `date:`; a 2030 range reported `horizonLeft`.
- That run found `search location:ada` failing with `bm25` called outside its full-text query: a common table expression holding a body-only match was flattened into the statement reading `rank`. Text-term tables are now `MATERIALIZED`, and pimdir gained a case per field with a check that none lacks one.

## Not done

- M3 (threads, relative dates) and M4 (`tag`, `thread`).
- A calendar component with no `UID` yields no occurrence (the timeline is addressed by `UID`). `has:attachment` reads the mail mark alone; calendar `ATTACH` names are searchable as `attachment:` but do not set it. Floating and all-day times read on UTC, with no option for another indexer zone. Between horizon rolls `horizonLeft` judges against the recorded horizon, up to a day behind the clock.

## Capabilities moved

- search: "Calendar time is expanded through ical-rs within a rolling horizon" (added); the extraction, query and summary paragraphs (modified).
