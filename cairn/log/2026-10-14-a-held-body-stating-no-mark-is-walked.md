---
cairn: log
change: a-held-body-stating-no-mark-is-walked
date: 2026-10-14
---

# A held body stating no mark is walked for its columns

linked-and-streamed-body-columns kept the writer's mark on a placement linked to a held body when no held row stated one. Rows written before marks were read off bodies all hold `NULL` (every store MOA filled has many), and neverest is about to stop walking bodies itself, so such a copy, move or second label would read its listing's guess for good. Implements pimdir's a-held-body-stating-no-mark-is-walked (Annex A.1, draft-04, untagged).

## What landed

- `client::attachments::gaining`: in the held case, when `held_mail_summary` answers no mark, `walked_columns` walks the body from its blob with the same bounded `walk` and states its mark and invitation on the new row (the walk's invitation, else the held row's). Its parts stand for nothing: a body held before is never walked for stand-ins (§14.3), and the held rows are not rewritten. A body the blob store lacks keeps the writer's mark, with a warning. Only a `NULL` mark triggers the walk, a `NULL` invitation being also the value of a body inviting to nothing.

## Verification

- tests/attachments.rs `a_copy_of_a_body_held_with_no_mark_takes_the_columns_a_walk_gives`: the held rows' mark and invitation set to `NULL` and their stand-ins dropped by SQL, as an older writer left them; a copy guessing the opposite mark takes the walk's mark (an attachment for the invoice, none for the invitation) and `UID`, no stand-in is recorded, and the held rows still state neither. It fails on ca4c589.
- `cargo fmt --check`, `cargo clippy --all-features --all-targets -- -D warnings`, `cargo test --all-features` (spec_drift against the pimdir checkout) green.

## Not done

- A held row whose mark was a listing's guess (non-`NULL`) still passes it on: nothing tells a guess from a read.

## Capabilities moved

- store: "A body gaining a row states the columns only the body gives" (modified).
