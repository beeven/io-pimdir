//! # Search SQL
//!
//! The index schema and statements of SEARCH.md, generated at build time
//! from the specification vendored under spec/ (migrations/search/ and
//! queries/search/), each prepared on index.db with the store attached
//! read-only as `store` (SEARCH §2).
//!
//! The statements of this crate's own follow: the indexer's bookkeeping
//! around the canonical ones, none of which another implementation needs
//! to run the same index.

include!(concat!(env!("OUT_DIR"), "/search.rs"));

/// The FTS5 `tokenize` string in force (SEARCH §5), recorded in
/// `index_meta.tokenizer`: an index recording another is rebuilt.
pub const TOKENIZER: &str = "unicode61 remove_diacritics 2";

/// Whether a collection was stamped above the cursor: a rename leaves the
/// old id's placements to the key reconciliation (SEARCH §4).
pub const COLLECTIONS_MOVED: &str =
    r#"SELECT EXISTS (SELECT 1 FROM store.collections WHERE changed > :since);"#;

/// The live placements the index holds without the body the store holds:
/// a blob the indexer could not read when it folded the row, met again on
/// the next pass (SEARCH §4). Files are indexed from their summary alone,
/// so they never wait. The columns are `load_changed_items`'.
pub const LOAD_UNREAD_BODIES: &str = r#"SELECT i.collection, i.link_id, i.seq, i.object_hash, i.flags, i.deleted, i.retained_at, c.kind, c.account, i.changed
FROM placement p
JOIN store.items i ON i.collection = p.collection AND i.seq = p.seq
JOIN store.collections c ON c.id = i.collection
WHERE p.hash IS NULL AND i.object_hash IS NOT NULL AND i.deleted = 0
  AND c.kind != 'application/octet-stream'
LIMIT :limit;"#;

/// The live items of the collections of a JSON array stamped above the
/// index cursor: what an answer may see stale or not at all.
pub const COUNT_UNINDEXED: &str = r#"SELECT count(*) FROM store.items
WHERE changed > :since AND deleted = 0 AND retained_at IS NULL
  AND collection IN (SELECT value FROM json_each(:collections));"#;

/// Every live placement of one hit, `(account, seq)` (SEARCH §8.2), with
/// its kind's component and title, in collection order.
pub const LOAD_HIT_PLACEMENTS: &str = r#"SELECT i.collection, i.link_id, i.flags, i.object_hash, i.sort_key, i.level, c.kind,
       CASE c.kind
         WHEN 'message/rfc822' THEN 'mail'
         WHEN 'text/vcard' THEN 'contact'
         WHEN 'application/octet-stream' THEN 'file'
         WHEN 'text/calendar' THEN CASE
           WHEN e.link_id IS NOT NULL THEN 'event'
           WHEN t.link_id IS NOT NULL THEN 'task'
           WHEN j.link_id IS NOT NULL THEN 'journal'
         END
       END,
       coalesce(m.subject, k.fn, e.summary, t.summary, j.summary, f.name, '')
FROM store.items i
JOIN store.collections c ON c.id = i.collection
LEFT JOIN store.mail_summary m ON m.collection = i.collection AND m.link_id = i.link_id
LEFT JOIN store.contact_summary k ON k.collection = i.collection AND k.link_id = i.link_id
LEFT JOIN store.event_summary e ON e.collection = i.collection AND e.link_id = i.link_id
LEFT JOIN store.task_summary t ON t.collection = i.collection AND t.link_id = i.link_id
LEFT JOIN store.journal_summary j ON j.collection = i.collection AND j.link_id = i.link_id
LEFT JOIN store.file_summary f ON f.collection = i.collection AND f.link_id = i.link_id
WHERE i.seq = :seq AND c.account IS :account AND i.deleted = 0 AND i.retained_at IS NULL
ORDER BY i.collection;"#;

/// The item an `id:` value names, every kind it is held under (SEARCH §8):
/// by `seq` or by link id, its live placements' kinds and keys, so
/// `linked` is asked once per pair.
pub const LOAD_ENDS: &str = r#"SELECT DISTINCT c.kind, i.link_id FROM store.items i
JOIN store.collections c ON c.id = i.collection
WHERE (:seq IS NULL OR i.seq = :seq) AND (:link_id IS NULL OR i.link_id = :link_id)
  AND i.deleted = 0 AND i.retained_at IS NULL;"#;

/// The body the index holds for one placement, `NULL` while bodiless: what
/// its occurrences are expanded from (SEARCH §7).
pub const PLACEMENT_HASH: &str =
    r#"SELECT hash FROM placement WHERE collection = :collection AND seq = :seq;"#;

/// Every statement of this crate's own, paired with its constant name.
pub const OWN: &[(&str, &str)] = &[
    ("COLLECTIONS_MOVED", COLLECTIONS_MOVED),
    ("LOAD_UNREAD_BODIES", LOAD_UNREAD_BODIES),
    ("COUNT_UNINDEXED", COUNT_UNINDEXED),
    ("LOAD_HIT_PLACEMENTS", LOAD_HIT_PLACEMENTS),
    ("LOAD_ENDS", LOAD_ENDS),
    ("PLACEMENT_HASH", PLACEMENT_HASH),
];

/// Every search statement, canonical then this crate's own, paired with
/// its name.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str)> {
    CANONICAL.iter().chain(OWN).copied()
}
