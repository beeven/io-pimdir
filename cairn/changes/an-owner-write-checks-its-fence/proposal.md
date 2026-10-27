---
cairn: change
id: an-owner-write-checks-its-fence
status: draft
created: 2026-10-18
---

# An owner write checks its fence

## Why

Ownership is an advisory lock on owner.lock (pimdir STORAGE §8), taken once per process and shared by its handles (single-owner-lock, owner-lock-handover). The lock is checked when it is taken, never when a write commits. Two cases slip through:

- a mobile worker suspended mid-run and resumed after its lock was lost and taken by another process (a store on a network filesystem, where §8 already says advisory locks are unreliable and a lease SHOULD enforce a single instance, without saying how);
- a handle kept past a release by a bug in the caller, whose next write lands under another owner's feet.

Borrowed from an external engine's design: a monotonic fencing token is the write-admission gate itself, re-checked inside the write transaction, with a lease's expiry only a safety net.

## What

- `store_meta` carries an owner epoch, drawn up when a process takes the owner role.
- Every owner write transaction checks, inside the transaction, that the epoch is the one its process drew, and fails with a stale-owner error otherwise, before any row moves. A producer's enqueue is not an owner write and is not fenced.
- On a network filesystem, the epoch is what §8's lease enforces: a process whose lease lapsed finds the epoch moved and stops.

## Open questions

- The cost: one read of `store_meta` per write transaction, against a row already read on most paths.
- Whether pimdir STORAGE §8 makes the fence a MUST for every owner or a SHOULD for a store on a network filesystem only.
- How the Android store, which owns pimdir.db from Java, draws and checks the epoch.
