//! Stand-ins recorded as a message's body enters the store (STORAGE
//! §14.3): one file per part carrying `Content-Disposition: attachment`,
//! at `Meta` in the account's attachments collection, referenced from
//! the message, in the transaction storing the body, whatever path
//! stores it, once.

use std::{io::Write, path::Path};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{PimdirSourceStore, PimdirStore, blobs::PimdirBlobs, producer::PimdirProducer},
    codec::PimdirAction,
    collection::PimdirCollectionId,
    hash::PimdirHashAlgo,
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirBase, PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    summary::{
        file::{self, PimdirFileSummary},
        mail,
    },
};

const MAIL: &str = "message/rfc822";
const ACCOUNT: &str = "work";
const ATTACHMENTS: &str = "work/\u{1}attachments";

/// A message with a text body, an inline image, a PDF and a text file
/// named under RFC 2231, nested one level.
const BODY: &[u8] = b"Message-ID: <invoice-7@example.org>\r\n\
From: Billing <billing@example.org>\r\n\
To: bob@example.org\r\n\
Subject: Invoice\r\n\
Date: Mon, 05 Oct 2026 09:00:00 +0000\r\n\
Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
\r\n\
--outer\r\n\
Content-Type: multipart/related; boundary=\"inner\"\r\n\
\r\n\
--inner\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Attached.</p><img src=\"cid:logo\">\r\n\
--inner\r\n\
Content-Type: image/png\r\n\
Content-Disposition: inline; filename=\"logo.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--inner--\r\n\
--outer\r\n\
Content-Type: application/pdf; name=\"invoice.pdf\"\r\n\
Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--outer\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Disposition: attachment; filename*=UTF-8''d%C3%A9tail.txt\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
total=3A 42=E2=82=AC\r\n\
--outer--\r\n";

const LINK: &str = "invoice-7@example.org";

fn hash_of(body: &[u8]) -> PimdirHash {
    PimdirHashAlgo::default().hash(body)
}

fn store_object(body: &[u8]) -> PimdirWriteOp {
    PimdirWriteOp::StoreObject {
        object: PimdirObject {
            hash: hash_of(body),
            size: body.len(),
        },
        body: Some(body.to_vec()),
    }
}

/// A clean placement of `link` under `handle` holding `body`, its
/// summary derived from it.
fn placement(collection: &str, handle: &str, link: &str, body: Option<&[u8]>) -> PimdirPlacement {
    let object = body.map(hash_of);
    let summary = mail::derive(body.unwrap_or(BODY)).summary;
    PimdirPlacement {
        sort_key: Default::default(),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link.into())),
        object: object.clone(),
        level: if object.is_some() {
            PimdirLevel::Full
        } else {
            PimdirLevel::Meta
        },
        summary,
        flags: PimdirFlags::default(),
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: Some(PimdirBase {
            flags: PimdirFlags::default(),
            revision: None,
            object,
        }),
        origin: None,
    }
}

fn opened(dir: &Path) -> PimdirSourceStore {
    let store = PimdirStore::open(dir)
        .unwrap()
        .for_account(ACCOUNT)
        .for_source("imap");
    store.ensure_collection("INBOX", MAIL).unwrap();
    store.ensure_collection("Archive", MAIL).unwrap();
    store
}

fn stored(store: &mut PimdirSourceStore, collection: &str, handle: &str, link: &str, body: &[u8]) {
    store
        .write(vec![
            store_object(body),
            PimdirWriteOp::UpsertPlacement(placement(collection, handle, link, Some(body))),
        ])
        .unwrap();
}

fn expected() -> Vec<PimdirFileSummary> {
    vec![
        PimdirFileSummary {
            name: "invoice.pdf".into(),
            media_type: Some("application/pdf".into()),
            size: Some(9),
            part: Some("2".into()),
        },
        PimdirFileSummary {
            name: "détail.txt".into(),
            media_type: Some("text/plain".into()),
            size: Some(12),
            part: Some("3".into()),
        },
    ]
}

/// The stand-ins of `link`: their keys, seqs and summaries, in order.
fn stand_ins(store: &PimdirSourceStore, link: &str) -> Vec<(String, i64, PimdirFileSummary)> {
    store
        .list_attachments(Some(ACCOUNT), &PimdirLinkId(link.into()))
        .unwrap()
        .into_iter()
        .map(|file| {
            assert_eq!(file.collection, ATTACHMENTS);
            assert_eq!(file.object, None, "a stand-in holds no body");
            (file.link_id.0, file.seq, file.summary.unwrap())
        })
        .collect()
}

/// Every row of the attachments collection, with its summary.
fn collection_rows(store: &PimdirSourceStore) -> Vec<String> {
    let mut rows: Vec<String> = store
        .list_items(ATTACHMENTS, None, 100)
        .unwrap()
        .into_iter()
        .map(|item| {
            format!(
                "{}|{}|{:?}|{:?}",
                item.link_id.0, item.seq, item.object, item.level
            )
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn a_stored_body_stands_for_its_attachments() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    stored(&mut store, "INBOX", "1", LINK, BODY);

    let found = stand_ins(&store, LINK);
    assert_eq!(
        found
            .iter()
            .map(|(key, _, _)| key.as_str())
            .collect::<Vec<_>>(),
        [
            "part:invoice-7@example.org#2",
            "part:invoice-7@example.org#3"
        ]
    );
    assert_eq!(
        found
            .into_iter()
            .map(|(_, _, summary)| summary)
            .collect::<Vec<_>>(),
        expected()
    );
    assert_eq!(mail::attachments::attachments(BODY), expected());

    // NOTE: the collection is the account's, found again by its role.
    let collection = store
        .list_collections_by_account(Some(ACCOUNT))
        .unwrap()
        .into_iter()
        .find(|collection| collection.id == ATTACHMENTS)
        .unwrap();
    assert_eq!(collection.kind, file::KIND);
    assert_eq!(collection.name, "attachments");
    assert_eq!(collection.role.as_deref(), Some("attachments"));

    let items = store.list_items(ATTACHMENTS, None, 10).unwrap();
    assert!(
        items
            .iter()
            .all(|item| item.level == PimdirLevel::Meta && item.object.is_none())
    );

    let references = store
        .references_from(&PimdirEndpoint {
            kind: MAIL.into(),
            link_id: PimdirLinkId(LINK.into()),
        })
        .unwrap();
    let attached: Vec<_> = references
        .iter()
        .filter(|reference| reference.role == PimdirReferenceRole::Attachment)
        .map(|reference| {
            assert_eq!(reference.origin, PimdirReferenceOrigin::Auto);
            assert_eq!(reference.to.kind, file::KIND);
            reference.to.link_id.0.as_str()
        })
        .collect();
    assert_eq!(
        attached,
        [
            "part:invoice-7@example.org#2",
            "part:invoice-7@example.org#3"
        ]
    );

    let listed = store
        .list_attachments_by_account(Some(ACCOUNT), None, 10)
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|file| file.message.0 == LINK));
    let searched = store
        .search_files(&[ATTACHMENTS], "%invoice%", None, 10)
        .unwrap();
    assert_eq!(searched.len(), 1);
}

#[test]
fn a_body_entering_again_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    stored(&mut store, "INBOX", "1", LINK, BODY);
    let before = collection_rows(&store);
    let files = stand_ins(&store, LINK);
    let message = PimdirEndpoint {
        kind: MAIL.into(),
        link_id: PimdirLinkId(LINK.into()),
    };
    let references = store.references_from(&message).unwrap();

    // NOTE: the same body restated, then filed under a second label.
    stored(&mut store, "INBOX", "1", LINK, BODY);
    stored(&mut store, "Archive", "9", LINK, BODY);
    assert_eq!(collection_rows(&store), before);

    // NOTE: released, then fetched again.
    store.release_before(&["INBOX", "Archive"], None).unwrap();
    assert_eq!(
        store.list_items("INBOX", None, 10).unwrap()[0].object,
        None,
        "the body was released"
    );
    stored(&mut store, "INBOX", "1", LINK, BODY);
    assert_eq!(collection_rows(&store), before);
    assert_eq!(stand_ins(&store, LINK), files);
    assert_eq!(store.references_from(&message).unwrap(), references);
}

#[test]
fn only_a_body_entering_is_walked_and_nothing_is_backfilled() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    // NOTE: named at Meta first: no body, no stand-in.
    store
        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
            "INBOX", "1", LINK, None,
        ))])
        .unwrap();
    assert!(stand_ins(&store, LINK).is_empty());

    // NOTE: the body arrives: that is the instant they are recorded.
    stored(&mut store, "INBOX", "1", LINK, BODY);
    assert_eq!(stand_ins(&store, LINK).len(), 2);

    // NOTE: a person unlinks both, which collects them; the body, held
    // already, filed again under another label brings nothing back.
    let message = PimdirEndpoint {
        kind: MAIL.into(),
        link_id: PimdirLinkId(LINK.into()),
    };
    for (key, _, _) in stand_ins(&store, LINK) {
        let file = PimdirEndpoint {
            kind: file::KIND.into(),
            link_id: PimdirLinkId(key),
        };
        store
            .remove_reference(&message, &file, &PimdirReferenceRole::Attachment)
            .unwrap()
            .unwrap();
    }
    assert!(stand_ins(&store, LINK).is_empty());
    assert!(collection_rows(&store).is_empty());
    stored(&mut store, "Archive", "9", LINK, BODY);
    assert!(stand_ins(&store, LINK).is_empty());
}

#[test]
fn no_stand_in_for_a_derived_key_inline_parts_or_another_kind() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());
    store.ensure_collection("Cards", "text/vcard").unwrap();

    // NOTE: the same parts, no Message-ID: keyed alt:, naming no identity.
    let anonymous: Vec<u8> = BODY
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.starts_with(b"Message-ID:"))
        .collect::<Vec<_>>()
        .join(&b'\n');
    let link = mail::derive(&anonymous).link_id;
    assert!(link.is_derived());
    stored(&mut store, "INBOX", "2", link.as_str(), &anonymous);
    assert!(stand_ins(&store, link.as_str()).is_empty());

    // NOTE: inline parts and a body with none disposed as attachments.
    let inline = b"Message-ID: <inline@example.org>\r\n\
Content-Type: multipart/related; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:x\">\r\n\
--b\r\nContent-Type: image/png\r\nContent-Disposition: inline; filename=x.png\r\n\r\nPNG\r\n\
--b--\r\n";
    stored(&mut store, "INBOX", "3", "inline@example.org", inline);
    assert!(stand_ins(&store, "inline@example.org").is_empty());

    // NOTE: a body of another kind is never walked as a message.
    store
        .write(vec![
            store_object(BODY),
            PimdirWriteOp::UpsertPlacement(placement("Cards", "c", "card-1", Some(BODY))),
        ])
        .unwrap();

    assert!(
        store
            .list_collections_by_account(Some(ACCOUNT))
            .unwrap()
            .iter()
            .all(|collection| collection.id != ATTACHMENTS),
        "nothing to stand for, no collection"
    );
}

#[test]
fn a_queued_add_stands_for_its_attachments_when_drained() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = opened(dir.path());

    let blobs = PimdirBlobs::open(dir.path(), PimdirHashAlgo::default());
    let mut writer = blobs.writer().unwrap();
    writer.write_all(BODY).unwrap();
    let hash = hash_of(BODY);
    let size = writer.commit(&hash).unwrap();
    let mut producer = PimdirProducer::open(dir.path(), "smtp").unwrap();
    producer
        .enqueue(
            "INBOX",
            &PimdirAction::Add {
                link_id: Some(PimdirLinkId(LINK.into())),
                flags: PimdirFlags::from_iter(["\\Draft"]),
                object: Some(hash.clone()),
            },
            Some(&PimdirObject {
                hash,
                size: size as usize,
            }),
        )
        .unwrap();

    let report = store.drain().unwrap();
    assert_eq!((report.applied, report.parked), (1, 0));
    assert_eq!(
        stand_ins(&store, LINK)
            .into_iter()
            .map(|(_, _, summary)| summary)
            .collect::<Vec<_>>(),
        expected()
    );
}

#[test]
fn a_store_of_one_account_files_them_under_no_account() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = PimdirStore::open(dir.path()).unwrap().for_source("imap");
    store.ensure_collection("INBOX", MAIL).unwrap();

    stored(&mut store, "INBOX", "1", LINK, BODY);

    let files = store
        .list_attachments(None, &PimdirLinkId(LINK.into()))
        .unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|file| file.collection == "\u{1}attachments")
    );
    let collection = store
        .list_collections_by_account(None)
        .unwrap()
        .into_iter()
        .find(|collection| collection.id == "\u{1}attachments")
        .unwrap();
    assert_eq!(collection.role.as_deref(), Some("attachments"));
}
