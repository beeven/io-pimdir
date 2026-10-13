---
cairn: log
change: round-head
date: 2026-10-13
---

# A round lists its head first

pimdir `round-head` (SYNC §4, §5; vectors/sync/62 to 69). A run resuming an open round asks for the round's head before its next page, so a message arriving during a long first round shows at the next run instead of when the round closes.

## What landed

- src/remote.rs:
  - `PimdirHeads` and `PimdirRemote::heads` (default none);
  - `PimdirHead` (`Delta`, `Band`) and `PimdirListing::Head`;
  - `PimdirEnumerated::HeadRejected` and `NoHead`;
  - `PimdirEnumerate::head` and `PimdirRemoteSnapshot::band_head`.

  It also documents what a connector owes: a delta head carries its checkpoint, pages after it are read fresh, and a band-head connector lists the band again in every round's last page.
- src/sync.rs:
  - `PimdirSync::heads` and the head planned when resuming;
  - the head merged as a page before the last, its candidates its members and vanished handles, no `Add` and no absence;
  - a delta head stamped, its checkpoint recorded as the round's with the cursor kept; a band head neither;
  - a rejected head restarts the round over the run's scope.
- src/change.rs, src/client/write.rs: `SetRoundCursor::cursor` is an `Option`; spec/ re-vendored, `set_round_cursor` coalescing the cursor. src/client/run.rs: `sync` passes the remote's heads.
- Tests: tests/vectors_sync.rs runs vectors 62 to 69 (and reads `round_started_at`); tests/scoped_sync.rs covers an arrival during a round through the std runner; unit tests in src/sync.rs.
- Capabilities moved: sync ("A resumed round lists its head first", added), seam ("A listed member carries its meta", appended).
