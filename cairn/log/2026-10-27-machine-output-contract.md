---
cairn: log
change: machine-output-contract
date: 2026-10-27
---

# Adopted the shared machine output contract

pimalaya-cli is pinned at `55b6693` until 0.3.0. `json-schema` gained `pimdir-error`, the schema of the JSON failure object, and each schema now carries `$id` = `https://pimalaya.org/schemas/pimdir/v0.8/<key>.json`. A test asserts every payload root is an object. `collection list` was the one that was not: it printed a bare array and now prints `{"collections": [...]}`, a breaking change for a consumer of its JSON. Piped `--json` is now one value per line, from the pimalaya-cli printer.

Verified: 316 binary tests pass with all features, `--no-default-features` builds. In the same working tree, the uncommitted search work fails `every_search_vector_answers` (tests/search.rs, a `date:` vector) and warns on `OCCURRENCES_MAX` and a complex type under clippy; neither touches this change.

Spec updated: cairn/spec/cli.md (ADDED: The JSON contract is described and versioned).
