---
cairn: log
change: search-feature
date: 2026-10-27
---

# Tagging and the thread verb, milestone M4

The writes of SEARCH §10 and the last verb: the index now tags through the queue and answers an item's thread. Task 2 of the 2027 NLnet plan ("pimdir Notmuch") has every milestone it planned in io-pimdir; the change stays active for its measured targets and pimdir's leaving draft.

## What landed

- client::search::tag: `PimdirIndex::tag` plans a flag change over a query's hits without writing, each placement's next flags starting from its last pending `set-flags` so tags staged in a row compose; `PimdirSearchTagPlan::stage` stages one `set-flags` per placement that changes. The source is resolved per placement from the sources syncing its collection (`ambiguousSource`, `unsynced` otherwise), a mail source lacking the capability the change needs is `unsupported` with what is `missing`, and `syncs` says whether the tag reaches the server: a contact or calendar tag stays in the store.
- `PimdirIndex::thread(account, seq)` through `thread_of` and `thread_members`.
- CLI `tag QUERY --add FLAG --remove FLAG [--dry-run]` (`pimdir-tag`): it drains when the owner role is free and reads each placement's outcome back from the store. CLI `thread SEQ --account ACCOUNT` (`pimdir-thread`).

## Verification

- Against pimdir 42d2d2a (before the GTD commit 1e59706, which adds `status:` and task summary columns io-pimdir does not answer yet), in a clean copy: `cargo test` default 576, `--features search` 603, `--all-features` 606, all passing; clippy clean on the three feature sets.
- tests/search.rs and tests/search_cli.rs: one action per changing placement of a message held in two collections, the drain applying it, an identical second tag staging nothing; dry run and a refused query writing nothing; queued while the owner lock is held, applied once free; one command over sources syncing different collections with no `--source`, an ambiguous collection; `syncs` and `unsupported` per source declaration, with a declaration changed between plan and stage refusing the stage whole; thread from a reply, a root and a file.
- End to end on Stalwart and Radicale synced by neverest: `tag --add flagged --add urgent` reached the IMAP server as `\Flagged` and the keyword, `--remove urgent` took it off. That run found two faults now fixed: `tag` refused a store syncing several sources until given one `--source`, and a tag on a CalDAV event read `applied` while neverest warned on every sync that CalDAV has no flags.

## Not done

- neverest warns on every sync about a store-local flag on a calendar item rather than recognising it as local.
- `status:` and the GTD task fields from pimdir 1e59706; re-vendoring; the measured targets; SEARCH.md leaving draft.

## Capabilities moved

- search: "Tagging stages through the queue, each placement as its own source" and "An item's thread is answered from the index" (added); the introduction and the hits requirement (modified).
- cli: "Item mutations enqueue, then drain when the owner role is free" (modified: `tag`), the verb surface (modified).
