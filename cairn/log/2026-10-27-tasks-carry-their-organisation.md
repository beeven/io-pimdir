---
cairn: log
change: tasks-carry-their-organisation
date: 2026-10-27
---

# Tasks carry their organisation

Follows pimdir 1e59706 and 7098741: spec/ copied again, and the crate meets them.

- **summaries**: `PimdirTaskSummary` gains `priority`, `categories`, `parent` and `mid` (Annex A.4), derived in src/summary/calendar.rs (`split_unescaped` generalises the structured-value split of src/summary/contact.rs), written by `upsert_task_summary` with `categories` as JSON, read by `get_task`, `list_tasks_page_asc` and `load_task_summaries`, printed by the CLI. Guarded by summaries.json's "todo, organised" (tests/summaries.rs) and a unit test of the edges.
- **store**: `link_references` runs `link_sources_to` for a mail, `link_parents_to` for every calendar item, `link_parents_of` and `link_sources_of` for a task naming a parent or a `mid`; `PimdirReferenceRole` gains `Parent` and `Source`, which `linked:` reads as roles. Opening an earlier store re-derives the four columns from the bodies `list_held_tasks` pages, then runs the four rules over everything. Guarded by tests/references.rs `a_task_refers_to_its_project_and_its_mail` and `an_earlier_store_derives_its_tasks_organisation_on_open`, both failing with the `link_sources_to` call or the backfill removed.
- **search**: `status:`, `priority:` and `category:` parsed and compiled to `store.task_summary`. `category:` folds both sides as Annex A.2 folds `fn`; SQLite's `lower()` folds ASCII alone, so every search connection registers `pimdir_fold`, and the `search` feature now enables rusqlite's `functions`. Guarded by the eleven new cases of queries.json (tests/search.rs) and a unit test of the fold.

Before this, every_search_vector_answers, two spec_drift tests and every_summary_vector_derives failed against the live spec; the whole suite and clippy pass.
