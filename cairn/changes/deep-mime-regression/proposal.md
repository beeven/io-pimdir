---
cairn: change
id: deep-mime-regression
status: active
created: 2026-10-18
---

# The 8000-level crash input as a regression test

## Why

The Android audit's S3 crash input io-pimdir-derive-mime-deep-8000.eml (548,722 bytes) overflowed the stack in `summary::mail::has_attachment` through `derive`. the-walk-of-a-body-is-bounded (2026-10-11) fixed it with `MIME_DEPTH_MAX` (64), and a probe of the input passes. tests/deep_nesting.rs pins the fix with 10,000 levels, but only with boundaries `b{level}x`, built so that none is a prefix of another; the crash input's boundaries are `b0` to `b7999`, where `b1` is a prefix of `b10` to `b19` and `b100` onwards. A delimiter search that matched a prefix would open the wrong part at every level, a path the current test never walks.

The input folder of the audit is about to be deleted and fuzz artifacts are not committed here, so the case must be regenerated in the test.

## What

One generated case in tests/deep_nesting.rs, on the 1 MiB thread the other cases use: the crash input's shape, rebuilt rather than committed.

The shape, as the input has it: headers `From: a@x` and `To: b@y`, no `Message-ID`, CRLF line endings; then for each level from 0 to 7999 `Content-Type: multipart/mixed; boundary=b{level}`, a blank line and `--b{level}`; then a leaf `Content-Type: text/plain`, a blank line and `hi`; then `--b{level}--` for each level from 7999 down to 0.

The expectation: `mail::derive` returns a mail summary within the stack, its attachment mark `Some(false)` and no invitation, as the 10,000-level case does.
