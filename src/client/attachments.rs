//! # Stand-ins, recorded as a body enters
//!
//! A message's attachments are files holding no body, one per part
//! carrying `Content-Disposition: attachment`, each referenced by the
//! message (STORAGE §14.3). They are recorded at the instant the
//! message's body enters the store, in the transaction writing it,
//! whatever wrote it: a sync landing a fetched body, an upgrade, a
//! queued add, a direct write. A body the store already held under the
//! same key in the account (a move, a copy, a second label) enters
//! nothing, so neither does a body held before the rule ran: nothing is
//! walked twice, nothing is backfilled, and a stand-in once recorded is
//! never recomputed.

use alloc::{format, string::String, vec::Vec};

use std::{collections::BTreeSet, io::Read};

use rusqlite::{Connection, OptionalExtension, named_params};

use crate::{
    client::{PimdirError, blobs::PimdirBlobs, write::kind_of},
    hub::PimdirHub,
    object::PimdirHash,
    placement::PimdirLinkId,
    sql,
    summary::{
        file::{self, PimdirFileSummary},
        mail::attachments::PimdirAttachmentWalk,
    },
};

/// The kind whose bodies stand for attachments.
const MAIL: &str = "message/rfc822";

/// The role of an account's attachments collection (STORAGE §14).
pub(crate) const ROLE: &str = "attachments";

/// The name an account's attachments collection is created under, after
/// the account and a `U+0001`, which no server puts in a mailbox name:
/// `<account>/\u{1}attachments`, or `\u{1}attachments` in a store of one
/// account. The Android app files them alike.
pub(crate) const NAME: &str = "attachments";

/// How much of a body one read takes.
const CHUNK: usize = 64 * 1024;

/// The bodies a batch brings into one mail collection, read before its
/// diff is saved.
pub(crate) struct Entering {
    collection: String,
    account: Option<String>,
    bodies: Vec<(PimdirLinkId, PimdirHash)>,
}

/// The bodies the diff of `collection` from `old` to `new` brings into
/// the store: a live, stored item under a key naming an identity gaining
/// a body no row of that key in the account held before the batch.
///
/// `None` for a collection of another kind, or when nothing enters.
pub(crate) fn entering(
    conn: &Connection,
    collection: &str,
    old: &PimdirHub,
    new: &PimdirHub,
) -> Result<Option<Entering>, PimdirError> {
    let mut bodies: Vec<(PimdirLinkId, PimdirHash)> = new
        .items
        .iter()
        .filter_map(|(link, item)| {
            let prev = old.items.get(link);
            let object = item.object.as_ref()?;
            let stored = prev.is_some() || !item.sources.is_empty();
            let moved = prev.and_then(|prev| prev.object.as_ref()) != Some(object);
            (stored && moved && !item.deleted && !link.is_derived())
                .then(|| (link.clone(), object.clone()))
        })
        .collect();
    if bodies.is_empty() || kind_of(conn, collection)? != MAIL {
        return Ok(None);
    }

    let account: Option<String> = conn
        .query_row(
            sql::LOAD_ACCOUNT,
            named_params! { ":collection": collection },
            |row| row.get(0),
        )
        .optional()?
        .flatten();

    // NOTE: lookup_objects answers the bodies a link id already has in
    // the account, which is what a move, a copy or a second label carry.
    let links: Vec<&str> = bodies.iter().map(|(link, _)| link.as_str()).collect();
    let mut stmt = conn.prepare(sql::LOOKUP_OBJECTS)?;
    let held: BTreeSet<(String, String)> = stmt
        .query_map(
            named_params! {
                ":links": serde_json::to_string(&links)?,
                ":account": account.as_deref(),
            },
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .collect::<Result<_, _>>()?;
    bodies.retain(|(link, hash)| !held.contains(&(link.0.clone(), hash.0.clone())));

    Ok((!bodies.is_empty()).then(|| Entering {
        collection: collection.into(),
        account,
        bodies,
    }))
}

/// Records the stand-ins of the bodies entering, once the diff that
/// stores them is saved: per part a file at `Meta` in the account's
/// attachments collection, created on demand, and the `attachment`
/// reference from the message. A part the collection holds already
/// keeps its stand-in.
pub(crate) fn record(
    conn: &Connection,
    blobs: &PimdirBlobs,
    entering: Entering,
) -> Result<(), PimdirError> {
    let mut attachments: Option<Option<String>> = None;

    for (message, hash) in entering.bodies {
        let parts = match walk(blobs, &hash)? {
            Some(parts) if !parts.is_empty() => parts,
            Some(_) => continue,
            None => {
                log::warn!(
                    "body {} of {} in {} is not in the blob store, no attachment recorded",
                    hash.0,
                    message.0,
                    entering.collection,
                );
                continue;
            }
        };
        let collection = match &attachments {
            Some(collection) => collection.clone(),
            None => {
                let found = collection_of(conn, entering.account.as_deref())?;
                attachments = Some(found.clone());
                found
            }
        };
        let Some(collection) = collection else {
            continue;
        };

        for summary in parts {
            let Some(section) = summary.part.as_deref() else {
                continue;
            };
            let Some(key) = file::part_key(&message, section) else {
                continue;
            };
            let held: Option<i64> = conn
                .query_row(
                    sql::SEQ_BY_LINK,
                    named_params! { ":collection": collection, ":link_id": key.0 },
                    |row| row.get(0),
                )
                .optional()?;
            if held.is_none() {
                insert_file(conn, &collection, &key, &summary)?;
            }
            conn.query_row(
                sql::ADD_REFERENCE,
                named_params! {
                    ":from_kind": MAIL,
                    ":from_link_id": message.0,
                    ":to_kind": file::KIND,
                    ":to_link_id": key.0,
                    ":role": "attachment",
                    ":origin": "auto",
                },
                |_| Ok(()),
            )
            .optional()?;
        }
    }

    Ok(())
}

/// The attachments of a stored body, read a chunk at a time and no
/// further than the walk needs; `None` when the blob store lacks it.
fn walk(
    blobs: &PimdirBlobs,
    hash: &PimdirHash,
) -> Result<Option<Vec<PimdirFileSummary>>, PimdirError> {
    let Some(mut file) = blobs.reader(hash)? else {
        return Ok(None);
    };
    let mut walk = PimdirAttachmentWalk::new();
    let mut buffer = alloc::vec![0; CHUNK];
    while !walk.done() {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.into()),
        };
        walk.feed(&buffer[..read]);
    }
    Ok(Some(walk.finish()))
}

/// The account's attachments collection: the file collection holding the
/// role, else the one under [`NAME`], declared and given the role here.
/// `None` when that id is taken by a collection of another kind.
fn collection_of(conn: &Connection, account: Option<&str>) -> Result<Option<String>, PimdirError> {
    let mut stmt = conn.prepare(sql::LIST_COLLECTIONS_BY_ACCOUNT)?;
    let held: Option<String> = stmt
        .query_map(named_params! { ":account": account }, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .find(|(_, kind, role)| kind == file::KIND && role.as_deref() == Some(ROLE))
        .map(|(id, _, _)| id);
    if held.is_some() {
        return Ok(held);
    }

    let id = match account {
        Some(account) => format!("{account}/\u{1}{NAME}"),
        None => format!("\u{1}{NAME}"),
    };
    let declared: Option<String> = conn
        .query_row(sql::LOAD_KIND, named_params! { ":collection": id }, |row| {
            row.get(0)
        })
        .optional()?;
    if declared.is_some_and(|kind| !kind.is_empty() && kind != file::KIND) {
        return Ok(None);
    }
    conn.execute(
        sql::SET_COLLECTION_KIND,
        named_params! { ":collection": id, ":account": account, ":kind": file::KIND },
    )?;
    conn.execute(
        sql::SET_COLLECTION_NAME,
        named_params! { ":collection": id, ":account": account, ":name": NAME },
    )?;
    conn.execute(
        sql::SET_COLLECTION_ROLE,
        named_params! { ":collection": id, ":role": ROLE },
    )?;
    Ok(Some(id))
}

/// Writes a file holding no body at `Meta` with its summary, sharing the
/// public id its key has elsewhere (§9.1), answering it.
pub(crate) fn insert_file(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    summary: &PimdirFileSummary,
) -> rusqlite::Result<i64> {
    let shared: Option<i64> = conn
        .query_row(
            sql::SEQ_FOR_LINK_ANY,
            named_params! { ":link_id": link.as_str() },
            |row| row.get(0),
        )
        .optional()?;
    let seq = match shared {
        Some(seq) => seq,
        None => conn.query_row(sql::BUMP_NEXT_SEQ, [], |row| row.get(0))?,
    };
    conn.execute(
        sql::INSERT_ITEM,
        named_params! {
            ":collection": collection,
            ":link_id": link.as_str(),
            ":seq": seq,
            ":flags": "[]",
            ":object_hash": None::<String>,
            ":sort_key": summary.sort_key().0,
            ":level": 1,
            ":deleted": 0,
            ":conflicted": 0,
            ":conflict_object": None::<String>,
        },
    )?;
    upsert_file_summary(conn, collection, link, summary)?;
    Ok(seq)
}

/// Writes one placement's `file_summary` row.
pub(crate) fn upsert_file_summary(
    conn: &Connection,
    collection: &str,
    link: &PimdirLinkId,
    summary: &PimdirFileSummary,
) -> rusqlite::Result<()> {
    conn.execute(
        sql::UPSERT_FILE_SUMMARY,
        named_params! {
            ":collection": collection,
            ":link_id": link.as_str(),
            ":name": summary.name,
            ":media_type": summary.media_type,
            ":size": summary.size.map(|size| size as i64),
            ":part": summary.part,
        },
    )?;
    Ok(())
}
