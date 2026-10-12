---
cairn: log
change: the-walk-of-a-body-is-bounded
date: 2026-10-11
---

# The walk of a body is bounded

Found by the Android audit (S3, finding 1): `has_attachment` recursed once per multipart level, so a message nesting about 5000 levels (350 KB) overflowed a 1 MiB stack, a SIGSEGV no JNI guard catches, and every sync pass fetching it died again. `invitation` recursed the same way. The calendar derivation built its tree iteratively but nested it as deep as the body did, so dropping a body of some 100000 nested `BEGIN` lines overflowed too.

## What landed

- src/summary/mail.rs: `MIME_DEPTH_MAX` (64); `has_attachment` and `invitation` stop opening multipart bodies at it, which also bounds the rescan of the body each level costs.
- src/summary/calendar.rs: `parse` drops a component nested past the third level as it closes; no derivation reads deeper, so no column moves.
- tests/deep_nesting.rs: both trees on a 1 MiB thread (both overflowed before), and the bound itself.
- Capabilities moved: summaries ("The walk of a body is bounded in depth", added).

## Not done

pimdir STORAGE Annex A.1 says nothing of depth, so another writer walking deeper may mark a message this one does not. Proposed to the format: the walk opens multipart bodies down to 64 levels, the message at zero.
