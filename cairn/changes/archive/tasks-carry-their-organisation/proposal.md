---
cairn: change
id: tasks-carry-their-organisation
status: landed
created: 2026-10-27
---

# Tasks carry their organisation

## Why

pimdir 1e59706 and 7098741 (tasks-carry-their-organisation, a-store-upgrade-pages-its-tasks) gave `task_summary` four columns (`priority`, `categories`, `parent`, `mid`), two reference roles (`parent`, `source`) with four rule statements, a statement paging held task bodies for an upgrade, and three search fields (`status:`, `priority:`, `category:`). Pimalaya Android's GTD tasks (pimalaya/android cairn/changes/gtd) rest on them. Against the live spec, every_search_vector_answers, two spec_drift tests and every_summary_vector_derives failed here.

## What

- spec/ copied again from pimdir.
- `PimdirTaskSummary` gains the four fields, derived in src/summary/calendar.rs on Annex A.4's terms, written and read by the store and printed by the CLI.
- `link_references` runs the four rules; `PimdirReferenceRole` gains `Parent` and `Source`, and `linked:` reads both as roles.
- Opening an earlier store re-derives the four columns from the bodies `list_held_tasks` pages, then runs the four rules with a `NULL` link id.
- The three search fields parsed and compiled to `store.task_summary`; `category:` folds through a `pimdir_fold` SQL function registered on the search connection (the `search` feature now enables rusqlite's `functions`).
