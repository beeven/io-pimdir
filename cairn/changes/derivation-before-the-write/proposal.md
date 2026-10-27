---
cairn: change
id: derivation-before-the-write
status: draft
created: 2026-10-18
---

# Derive from a body before the write transaction opens

## Why

Borrowed from an external engine: compute the rows derived from a body before the store transaction, so the transaction stays mechanical and holds the writer lock only for writes.

The summary derivation is already pure and in the I/O-free core (summaries: `summary::derive`), and the search indexer reads the store without its lock. But the owner still reads and parses bodies inside its write transactions, re-verified on 2026-10-18:

- the queue drain derives an `add`'s and an `update`'s summary from the blob inside the apply transaction (src/client/producer.rs:722 and 847, `blobs.get` then `summary::derive` under `tx`);
- a write gaining a mail body walks it for its attachment mark, invitation and stand-ins inside the write (src/client/write.rs:365, `attachments::gaining(tx, blobs, ..)`).

A large message (the walk is bounded in depth, not in size) holds SQLite's single writer for as long as it takes to read and parse, and every producer and reader in WAL waits on the checkpoint behind it. On a phone the owner's write lock is also what the UI's own writes queue behind.

## What

- The drain reads the pending rows, derives every summary it will need outside any transaction, then applies each row in its own short transaction, re-checking under the transaction only what the derivation cannot know (the row still pending, the collection's kind).
- A write that gains a body takes the walk's result as input, computed by the caller before the batch (the engine already holds the body when it yields `StoreObject`), and the store only writes it.
- No change in what is derived or written: the vectors under vectors/summaries.json and the engine suites pin it.

## Open questions

- A body changing between the derivation and the write cannot happen (bodies are immutable by hash), but the item's kind can: re-check it in the transaction, or derive under a kind read with the row.
- Whether the drain derives in parallel, as SEARCH §4 allows the indexer.
