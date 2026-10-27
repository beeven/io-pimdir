---
cairn: change
id: raw-payloads-print-their-length
status: draft
created: 2026-10-18
---

# A raw body prints its length, never its content

## Why

Borrowed from an external engine: raw-payload newtypes carry a redacted `Debug` that prints the length only.

Here every type carrying a body derives `Debug` over a plain `Vec<u8>`, re-verified on 2026-10-18: `PimdirFetchedBody::Inline { bytes }` (src/remote.rs), `PimdirWriteOp::StoreObject { body }` (src/change.rs), the mutations' `body` (src/mutate.rs). The engine logs these structures at trace level (src/sync.rs:1036 `changes: {changes:?}`, :457 `enumerate {request:?}`, :1898 `push results`), so a trace log of a sync holds whole messages, cards and events, as byte arrays, in a file a user attaches to a bug report.

## What

- A `PimdirBytes` newtype for every raw body the crate's public types carry, whose `Debug` prints `PimdirBytes(<n> bytes)`, deref and conversion to `Vec<u8>` and `&[u8]` kept so callers change little.
- The trace lines keep logging the structures; they now show sizes.
- A test that the `Debug` of each body-carrying type contains no byte of a body.

## Open questions

- A breaking change to public field types: land it with the next breaking release, or keep `Vec<u8>` fields and hand-write `Debug` on the carrying types instead.
- The summary rows (subject, sender) are personal data too; whether trace logs should print them is a separate question.
