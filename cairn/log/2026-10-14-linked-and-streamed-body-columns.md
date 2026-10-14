---
cairn: log
change: linked-and-streamed-body-columns
date: 2026-10-14
---

# A body gaining a row states what only the body gives

Closes the two gaps neverest's log of 2026-10-13 left: a body linked from the store kept its listing's mark (and no invitation, and the server's size), and a body streamed to its blob had no invitation. Implements pimdir's linked-and-streamed-body-columns (draft-04, untagged): Annex A.1's rule for a linked placement and a streamed body, `held_mail_summary`.

## What landed

- `summary::mail::attachments`: the uncommitted `Calendar` reader a stopped agent left was reviewed and kept. It reads the first `UID` of the first `text/calendar` part stating one as the octets go by (base64 by its alphabet, a foreign byte voiding the part; quoted-printable escapes with no soft break undone, as `derive`; lines unfolded at a space or a tab; split at the first colon, the name before any parameter), keeping one decoded line and one unfolded line of at most `CALENDAR_LINE_MAX` (64 KiB): a longer `UID` reads as none and ends the search. A part no delimiter ends gives no invitation (`invited_before` on each frame, as for the stand-ins). Added to it: the mark in `PimdirParts` (`attachment`: a part recorded, or the message's own header block disposing a multipart as an attachment, which has no section and so no summary), and `parts(body)`. `done()` waits for an unsettled calendar part.
- `summary::mail`: `base64` and `quoted_printable` take octets; `invitation` decodes the part's octets, no longer a lossy string of them, so a non-UTF-8 octet beside an escape reads as the streamed walk reads it (the only inequivalence the review found).
- `client::attachments::gaining`, called by `client::write::apply` before the diff is saved, replaces `entering`: per body a mail item gains, a held one (`lookup_objects`) takes the object's size and the mark and invitation `held_mail_summary` answers, a mark no held row states keeping the writer's; an entering one is walked once from its blob, its mark and (when known) invitation set on the item's summary, its stand-ins kept for `record` unless its key is derived. Derived keys are now walked for their columns. `record` no longer walks. The summary patched in the hub, `save_hub_diff` writes it and runs `link_invitations_of`.
- spec/queries/storage/owner/held_mail_summary.sql, vendored from pimdir.

Paths that link a held body without a fetch, all through `apply`: the upgrade's `LookingUp` (src/upgrade.rs, which patches the object and keeps the loaded summary), a writer's own copy of a held hash (an IMAP COPY or MOVE, a second Gmail label, landed by neverest's `write`), a drained `add` by copy. The sync engine links nothing itself; a revive keeps the retained row's body, whose object does not move, and `summary::without_body` keeps the then corrected row on every later listing.

## Verification

- tests/walk_agrees.rs: a table (base64 wrapped, quoted-printable with escapes, an `=3D`, a soft break and a dangling escape, folded `UID` lines with a space, a tab and a fold at the end, a parameter on the name, nested alternatives with an inline calendar before an attached one, a first calendar part with no `UID`, a truncated message, an inner part cut short by an outer delimiter, a foreign base64 byte, a plain part stating a `UID`, raw non-UTF-8 beside escapes, a disposed top-level multipart) and a property of 512 messages built from those pieces at random (CRLF or LF, nesting to three levels, preambles, missing close delimiters, truncation anywhere), each fed whole, by 1, 7 and 64 KiB octets stopping when `done()`, and cut at a random point, against `derive`; the line bound.
- tests/summaries.rs: the walk's mark and invitation equal every vector's row.
- tests/attachments.rs: `a_body_read_without_it_states_what_its_parts_give`, `a_copy_of_a_held_body_takes_the_columns_its_body_gave`, `an_upgrade_linking_a_held_body_takes_the_columns_its_body_gave` (no fetch reached), each failing without the change to `apply`.
- `cargo fmt --check`, `cargo clippy --all-features --all-targets -D warnings`, `cargo clippy --no-default-features --lib`, `cargo test --all-features` (spec_drift against the pimdir checkout), `--no-default-features --features cli`, `--no-default-features --lib`, `cargo doc` green.

## Not done

- A held row whose mark was itself a listing's guess (a copy linked before this change) passes it on: nothing tells a guess from a read, and walking a held body again is what §14.3 rules out. A rebuild could re-derive them as it re-derives the invitation.
- neverest still sets the mark from its own walk and leaves the invitation unset; `apply` now states both, so it needs no code, but it could take `finish_parts` and drop its own reading.
- The bounds the streamed walk keeps (`HEADER_MAX`, a boundary over 1020 octets, `CALENDAR_LINE_MAX`) are this crate's, past which it and `derive` may differ; pimdir states none.

## Capabilities moved

- store: "A body gaining a row states the columns only the body gives" (added).
- summaries: "The walk of a message's parts is streamed and bounded" and "Decoding follows Annex A.0" (modified).
- upgrade: "A linked body carries the base with it" (modified).
