---
cairn: log
change: stand-ins-as-the-body-enters
date: 2026-10-13
---

# A body entering the store stands for its attachments

Implements pimdir's stand-ins-as-the-body-enters (draft-04, untagged): STORAGE §14.3 "Stand-ins are recorded as the body enters", Annex A.7's size and name, summaries.json's `attachments`, sync vector 70. No statement or schema change, so spec/ is unchanged.

- `summary::mail::attachments`: `PimdirAttachmentWalk` (`new`, `feed`, `done`, `finish`) and `attachments(body)`, the parts carrying `Content-Disposition: attachment` as `PimdirFileSummary` rows with their section. The walk is the mark's (the same `boundary`, disposition test, outermost delimiter first, a part no delimiter ends being none, `MIME_DEPTH_MAX`), line by line over chunks: `HEADER_MAX` (64 KiB) per header block, `LINE_HEAD` (1 KiB) per other line, one boundary per open level, `ATTACHMENTS_MAX` (1000) rows; the bytes of a part are counted, never kept (base64 by its alphabet, quoted-printable by RFC 2045 with soft breaks and trailing blanks, identity with the line break before a delimiter left to it). A single-part message is done after its header block, so a body with no multipart reads its header only.
- `client::write::apply`, per collection of `message/rfc822`: `client::attachments::entering` takes, before the diff is saved, the bodies a live, stored item under a non-derived key gains, less those `lookup_objects` finds held under the key in the account (a move, a copy, a second label, the same body again); `record` reads each from its blob file in 64 KiB chunks after the diff, and per part not held writes the stand-in (`insert_file`: `seq_for_link_any` or `bump_next_seq`, `insert_item` at level 1 with no body, `upsert_file_summary`) and `add_reference`, `attachment`, `auto`. The collection is the account's file collection with the role, else `<account>/\u0001attachments` (`\u0001attachments` without an account), declared by `set_collection_kind`, `set_collection_name` (`attachments`) and `set_collection_role`, as the Android app names it; a body missing from the blob store is logged and walks nothing.
- Every writer gets it: a sync landing a fetch, an upgrade, a drained `add` (himalaya's drafts), a direct `write`. neverest needs no code.
- `put_file` shares `insert_file` and `upsert_file_summary`.
- The rules (§14.2) already ran on every summary write; a test now pins that a link made at the write stays when its end changes.
- Capabilities moved: store ("A body entering the store stands for its attachments", added; "The writer records the automatic references", a scenario), summaries ("The walk of a message's parts is streamed and bounded", added).

## Verification

tests/attachments.rs (6): two stand-ins named, typed, measured and numbered, the collection's id, name and role, `list_attachments_by_account` and `search_files` over them; the same body restated, filed in Archive, released and fetched again write nothing (rows, seqs, summaries, references); a body named at `Meta` records nothing until its body enters, and stand-ins a person unlinked are not brought back by the body filed again; none for an `alt:` key, inline parts or a body in a card collection, and no collection declared; a drained `add`; a store with no account. tests/references.rs `a_rule_links_at_the_write_and_never_recomputes`. tests/summaries.rs checks every case's `attachments`, the walk fed whole and in 7-octet chunks; tests/vectors_sync.rs reads `references` and a collection's `name` and `role`, and vector 70 passes (it fails with a stand-in's seq altered). Unit tests: every two-chunk split and byte-by-byte feeding agree, a long line counted and a long delimiter line delimiting, the depth bound agreeing with the mark, the count bound. `cargo test --all-features`, `--no-default-features --features cli`, clippy `-D warnings`, `cargo doc` and `cargo fmt --check` green; `spec_drift` against the pimdir checkout.

## Not done

- The count bound (1000) and the depth bound (64) are this crate's; pimdir states neither, so another writer may stand for parts this one does not in a message no mail agent writes. Proposed to the format with the depth bound of 2026-10-11.
- The Android app still walks with mail-parser in `FileStore.recordAttachments`; switching its bridge to `summary::mail::attachments` makes the two walks one (mail-parser differs on malformed transfer encodings and on a part no delimiter ends).
- neverest derives a streamed body's summary from its header prefix and leaves the attachment mark unset; the walk here could restate it, which is A.1's job and not this change's.
