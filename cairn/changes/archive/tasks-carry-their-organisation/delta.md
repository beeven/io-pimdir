---
cairn: delta
change: tasks-carry-their-organisation
---

## ADDED Requirements

### Requirement: A task carries its organisation (summaries)
The four Annex A.4 columns derived, written, read and printed.

### Requirement: Tasks are searched by status, priority and category (search)
The three fields compiled to `store.task_summary`, `category:` folded by `pimdir_fold`.

## MODIFIED Requirements

### Requirement: The writer records the automatic references (store)
Also `link_sources_to` for a mail, `link_parents_to` for a calendar item, `link_parents_of` and `link_sources_of` for a task.

### Requirement: Open reconciles against the canonical text (store)
Also backfills the task columns by `list_held_tasks`, then the four task rules.

## REMOVED Requirements
