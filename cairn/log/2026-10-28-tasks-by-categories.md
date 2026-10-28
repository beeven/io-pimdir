---
cairn: log
change: tasks-by-categories
date: 2026-10-28
---

# Tasks by categories, and items kept on the device

Follows pimdir cairn/changes/tasks-by-categories (2026-10-28): the GTD convention as four queries over columns this crate already derives, and a local time collection no source syncs.

- **search**: the four view cases and the new fixture items (17 to 21, home/Tasks) pass through tests/search.rs `every_search_vector_answers` with no code change, a task stating no `STATUS` matching `-status:completed` as the pimdir vectors expect.
- **store**: `PimdirStore::put_item(collection, body)` writes a card or a calendar item into a collection no source syncs, deriving key, summary and sort key, and writing the item, summary, references and pins in one transaction (src/client/write.rs `put_unbound`, on the sync path's own `insert_item`, `write_summary`, `link_references` and `adjust_refcounts`); refused as `PimdirError::Synced` or `PimdirError::NotUnbound`. The proposal's `link_id` parameter was dropped: the key is the body's. `client::TIME_COLLECTION` and `ensure_time_collection` declare `\u{1}time`. Guarded by tests/unbound_items.rs, `a_collection_a_source_syncs_is_refused` failing with the refusal removed.
- spec/queries/storage/owner/delete_unbound_item.sql re-vendored (its comment names a time entry).

store.md gains "items kept on the device". unfog's local time strategy can now write through the crate.
