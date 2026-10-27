//! The search index against the format's own vectors (pimdir SEARCH.md
//! §11): the fixture store of vectors/search/store.json built through the
//! owner, indexed, and asked every case of vectors/search/queries.json;
//! the chunk boundaries of vectors/search/chunks.json.
//!
//! A case needing what this implementation does not answer yet is in
//! [`SKIPPED`] with its reason, and one naming a field the compiler
//! refuses must be refused as `unsupported-field`, never answered wrong.
//!
//! The spec is a sibling checkout, so the suite skips when it is absent.

#![cfg(feature = "search")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use io_pimdir::{
    change::PimdirWriteOp,
    client::{
        PimdirError, PimdirStore,
        search::{
            PimdirIndex, PimdirIndexRefreshOptions, PimdirSearchMatched, PimdirSearchOptions,
            PimdirSearchSort,
        },
    },
    collection::{PimdirCollectionId, PimdirScope},
    object::{PimdirHash, PimdirObject},
    placement::{
        PimdirFlags, PimdirHandle, PimdirLevel, PimdirLinkId, PimdirPlacement, PimdirSortKey,
        PimdirStatus,
    },
    reference::{PimdirEndpoint, PimdirReferenceOrigin, PimdirReferenceRole},
    search::{
        chunk,
        coverage::PimdirSearchTemporal,
        query::{PimdirQuery, PimdirQueryPredicate},
    },
    summary::{
        self, PimdirAddress, PimdirSummary, file::PimdirFileSummary, mail::PimdirMailSummary,
    },
};
use serde_json::Value;

/// The cases this implementation does not answer yet, by query, and why.
/// A case whose reason names a field is also checked to be refused as
/// `unsupported-field`.
const SKIPPED: &[(&str, &str)] = &[
    ("thread:multi-1@example.org", "M3: thread: (field)"),
    ("date:2w", "M3: relative dates (field)"),
    ("date:today..", "M3: relative dates (field)"),
];

fn spec_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("pimdir");
    dir.join("vectors/search/queries.json")
        .is_file()
        .then_some(dir)
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// One placement, the owner's write of what a source listed.
fn placement(
    collection: &str,
    handle: &str,
    link_id: &str,
    object: Option<PimdirHash>,
    summary: PimdirSummary,
    sort_key: &str,
    flags: PimdirFlags,
) -> PimdirPlacement {
    PimdirPlacement {
        sort_key: PimdirSortKey(sort_key.into()),
        collection: PimdirCollectionId(collection.into()),
        handle: PimdirHandle(handle.into()),
        link_id: Some(PimdirLinkId(link_id.into())),
        level: match object {
            Some(_) => PimdirLevel::Full,
            None => PimdirLevel::Meta,
        },
        object,
        summary: Some(summary),
        flags,
        status: PimdirStatus::Clean,
        conflict_revision: None,
        conflict_object: None,
        base: None,
        origin: None,
    }
}

/// A bodiless mail summary as a vector states it.
fn mail_summary(row: &Value, addresses: &[Value]) -> PimdirSummary {
    let role = |role: &str| -> Vec<PimdirAddress> {
        addresses
            .iter()
            .filter(|address| address["role"] == role)
            .map(|address| PimdirAddress {
                address: address["address"].as_str().unwrap().into(),
                name: address["name"].as_str().map(Into::into),
            })
            .collect()
    };
    let text = |key: &str| row[key].as_str().map(String::from);

    PimdirSummary::Mail(PimdirMailSummary {
        message_id: text("message_id"),
        in_reply_to: strings(&row["in_reply_to"]),
        subject: text("subject").unwrap_or_default(),
        sender: text("sender"),
        sender_name: text("sender_name"),
        date: text("date"),
        size: row["size"].as_u64(),
        attachment: row["attachment"].as_i64().map(|mark| mark != 0),
        invitation: text("invitation"),
        from: role("from"),
        to: role("to"),
        cc: role("cc"),
        bcc: role("bcc"),
    })
}

/// Builds the fixture store at `dir` through the owner, checking every
/// item draws the seq the vector names.
fn build(spec: &Path, dir: &Path) {
    let store = json(&spec.join("vectors/search/store.json"));

    let mut kinds = BTreeMap::new();
    let mut stand_ins = BTreeSet::new();
    for collection in store["collections"].as_array().unwrap() {
        let id = collection["id"].as_str().unwrap();
        let kind = collection["kind"].as_str().unwrap();
        let owner = PimdirStore::open(dir)
            .unwrap()
            .for_account(collection["account"].as_str().unwrap());
        owner.ensure_collection(id, kind).unwrap();
        if let Some(role) = collection["role"].as_str() {
            owner.set_collection_role(id, Some(role)).unwrap();
            if role == "attachments" {
                stand_ins.insert(id.to_string());
            }
        }
        kinds.insert(id.to_string(), kind.to_string());
        drop(owner);

        // NOTE: a coverage row is what a source's rounds leave (STORAGE
        // §10): one closed, or one still open when none ever closed.
        for source in collection["sources"].as_array().into_iter().flatten() {
            let scope = PimdirScope {
                since: source["covered_since"].as_str().map(Into::into),
                until: source["covered_until"].as_str().map(Into::into),
            };
            let mut ops = vec![PimdirWriteOp::OpenRound {
                collection: PimdirCollectionId(id.into()),
                scope: scope.clone(),
                band: false,
            }];
            if !source["covered_at"].is_null() {
                ops.push(PimdirWriteOp::CloseRound {
                    collection: PimdirCollectionId(id.into()),
                    coverage: scope,
                    checkpoint: None,
                });
            }
            PimdirStore::open(dir)
                .unwrap()
                .for_account(collection["account"].as_str().unwrap())
                .for_source(source["source"].as_str().unwrap())
                .write(ops)
                .unwrap();
        }
    }

    let mut store_handle = PimdirStore::open(dir).unwrap().for_source("vectors");
    for item in store["items"].as_array().unwrap() {
        let collection = item["collection"].as_str().unwrap();
        let link_id = item["link_id"].as_str().unwrap();
        let seq = item["seq"].as_i64().unwrap();
        let flags: PimdirFlags = strings(&item["flags"]).into_iter().collect();
        let kind = &kinds[collection];

        if stand_ins.contains(collection) {
            // NOTE: the owner records a stand-in as its message's body
            // enters (STORAGE §14.3); the vector states what it left.
        } else if let Some(fixture) = item["fixture"].as_str() {
            let body = fs::read(spec.join("vectors").join(fixture)).unwrap();
            let derived = summary::derive(kind, &body).unwrap();
            assert_eq!(derived.link_id.as_str(), link_id, "{fixture}: link id");
            let hash = store_handle.hash(&body);
            store_handle
                .write(vec![
                    PimdirWriteOp::StoreObject {
                        object: PimdirObject {
                            hash: hash.clone(),
                            size: body.len(),
                        },
                        body: Some(body),
                    },
                    PimdirWriteOp::UpsertPlacement(placement(
                        collection,
                        &seq.to_string(),
                        link_id,
                        Some(hash),
                        derived.summary.unwrap(),
                        derived.sort_key.as_str(),
                        flags,
                    )),
                ])
                .unwrap();
        } else {
            let stated = &item["summary"];
            let row = &stated["row"];
            match stated["table"].as_str().unwrap() {
                "mail_summary" => {
                    let addresses = stated["addresses"].as_array().unwrap();
                    store_handle
                        .write(vec![PimdirWriteOp::UpsertPlacement(placement(
                            collection,
                            &seq.to_string(),
                            link_id,
                            None,
                            mail_summary(row, addresses),
                            stated["sort_key"].as_str().unwrap(),
                            flags,
                        ))])
                        .unwrap();
                }
                "file_summary" => {
                    let file = PimdirFileSummary {
                        name: row["name"].as_str().unwrap().into(),
                        media_type: row["media_type"].as_str().map(Into::into),
                        size: row["size"].as_u64(),
                        part: row["part"].as_str().map(Into::into),
                    };
                    store_handle
                        .put_file(collection, &PimdirLinkId(link_id.into()), &file)
                        .unwrap();
                }
                table => panic!("no fixture builder for {table}"),
            }
        }

        let placed = store_handle.seq_for_link(collection, link_id).unwrap();
        assert_eq!(placed, Some(seq), "{link_id} draws the vector's seq");
    }

    for reference in store["references"].as_array().unwrap() {
        let end = |kind: &str, link: &str| PimdirEndpoint {
            kind: reference[kind].as_str().unwrap().into(),
            link_id: PimdirLinkId(reference[link].as_str().unwrap().into()),
        };
        store_handle
            .add_reference(
                &end("from_kind", "from_link_id"),
                &end("to_kind", "to_link_id"),
                &PimdirReferenceRole::parse(reference["role"].as_str().unwrap()),
                PimdirReferenceOrigin::parse(reference["origin"].as_str().unwrap()),
            )
            .unwrap()
            .expect("the reference links");
    }
}

/// A hit list as `(account, seq)` pairs.
fn pairs(hits: &Value) -> Vec<(String, i64)> {
    hits.as_array()
        .unwrap()
        .iter()
        .map(|hit| {
            (
                hit[0].as_str().unwrap().to_string(),
                hit[1].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn every_search_vector_answers() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(&spec, dir.path());
    let vectors = json(&spec.join("vectors/search/queries.json"));

    // NOTE: the horizon is drawn around the vectors' now (SEARCH §11).
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions {
            now: Some(unix(vectors["now"].as_str().unwrap())),
            ..Default::default()
        })
        .unwrap();
    assert!(report.is_current(), "one pass folds the whole feed");
    let coverage: BTreeMap<String, u64> = vectors["coverage"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(kind, count)| (kind.clone(), count.as_u64().unwrap()))
        .collect();
    let store = json(&spec.join("vectors/search/store.json"));
    assert_eq!(
        bodiless(&store, &PimdirQuery::parse("changed:0..").unwrap()),
        coverage,
        "the store-wide coverage the vectors state"
    );
    let cases = vectors["cases"].as_array().unwrap();
    assert!(!cases.is_empty(), "the vectors carry no case");

    let mut skipped = Vec::new();
    for case in cases {
        let query = case["query"].as_str().unwrap();

        if let Some(error) = case.get("error") {
            let refused = PimdirQuery::parse(query).expect_err(query);
            assert_eq!(
                (refused.code.as_str(), refused.start, refused.end),
                (
                    error["code"].as_str().unwrap(),
                    error["start"].as_u64().unwrap() as usize,
                    error["end"].as_u64().unwrap() as usize,
                ),
                "{query:?}: refusal"
            );
            if refused.code.as_str() == "unknown-field" {
                assert!(!refused.fields.is_empty(), "{query:?}: the known fields");
            }
            continue;
        }

        let parsed = PimdirQuery::parse(query).unwrap_or_else(|err| panic!("{query:?}: {err}"));

        if case.get("threads").is_some() {
            skipped.push((query, "M3: threaded listing"));
            continue;
        }
        if let Some((_, reason)) = SKIPPED.iter().find(|(skip, _)| *skip == query) {
            if reason.contains("(field)") {
                match index.search(&parsed, &PimdirSearchOptions::default()) {
                    Err(PimdirError::Query(err)) => {
                        assert_eq!(err.code.as_str(), "unsupported-field", "{query:?}")
                    }
                    other => panic!("{query:?}: refused as unsupported, got {other:?}"),
                }
            }
            skipped.push((query, reason));
            continue;
        }

        let options = PimdirSearchOptions {
            sort: match case["sort"].as_str() {
                Some("name") => PimdirSearchSort::Name,
                Some("relevance") => PimdirSearchSort::Relevance,
                _ => PimdirSearchSort::Date,
            },
            limit: 100,
            offset: 0,
        };
        let answer = index
            .search(&parsed, &options)
            .unwrap_or_else(|err| panic!("{query:?}: {err}"));
        let got: Vec<(String, i64)> = answer
            .hits
            .iter()
            .map(|hit| (hit.account.clone().unwrap_or_default(), hit.seq))
            .collect();
        let want = pairs(&case["hits"]);

        if case["ordered"].as_bool() == Some(true) {
            assert_eq!(got, want, "{query:?}: ordered hits");
        } else {
            assert_eq!(
                got.iter().collect::<BTreeSet<_>>(),
                want.iter().collect::<BTreeSet<_>>(),
                "{query:?}: hits"
            );
        }

        let count = index.count(&parsed).unwrap();
        assert_eq!(count.count as usize, want.len(), "{query:?}: count");

        let expected = bodiless(&store, &parsed);
        assert_eq!(
            answer.coverage.local.bodiless, expected,
            "{query:?}: coverage"
        );
        assert_eq!(answer.coverage.local.unindexed, 0, "{query:?}: unindexed");
        if let Some(temporal) = case.get("temporal") {
            let expected = match temporal.as_str() {
                Some("full") => PimdirSearchTemporal::Full,
                _ => PimdirSearchTemporal::Bounded {
                    since: temporal["since"].as_str().map(Into::into),
                    until: temporal["until"].as_str().map(Into::into),
                    filling: temporal["filling"].as_bool().unwrap(),
                },
            };
            assert_eq!(answer.coverage.temporal, expected, "{query:?}: temporal");
        }

        assert_eq!(
            answer.coverage.horizon_left,
            case["horizon_left"].as_bool().unwrap_or(false),
            "{query:?}: horizon_left"
        );
    }

    for (query, reason) in &skipped {
        eprintln!("skipped {query:?}: {reason}");
    }
}

#[test]
fn a_hit_says_what_it_matched_and_carries_a_snippet() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(&spec, dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();

    let search = |query: &str| {
        index
            .search(
                &PimdirQuery::parse(query).unwrap(),
                &PimdirSearchOptions::default(),
            )
            .unwrap()
    };

    let budget = search("budget");
    assert_eq!(budget.hits[0].matched, Some(PimdirSearchMatched::Summary));
    assert_eq!(budget.hits[0].snippet, None, "a summary has no chunk");

    let reply = search("final");
    assert_eq!(reply.hits[0].matched, Some(PimdirSearchMatched::Body));
    assert_eq!(
        reply.hits[0].snippet.as_deref(),
        Some("Roadmap roadmap roadmap. The roadmap is not final.")
    );

    let structured = search("is:flagged");
    assert_eq!(structured.hits[0].matched, None, "no text term");
    assert!(
        structured.hits[0].snippet.is_some(),
        "the first chunk stands in"
    );

    let item = index.item(Some("home"), 13).unwrap().unwrap();
    assert_eq!(item.source, PimdirSearchMatched::Body);
    assert!(item.extract.attachment.contains("invoice.pdf"));
    assert_eq!(item.extract.body.trim(), "Attached.");
    assert!(index.item(Some("work"), 13).unwrap().is_none());

    let card = index.item(Some("home"), 5).unwrap().unwrap().extract;
    assert_eq!(card.title, "Doe, Jane\nJane\nDoe");
    assert_eq!(card.org, "Example, Inc.\nResearch");
    assert_eq!(card.email, "Jane.Doe@Example.COM");
    assert_eq!(
        card.note,
        "+33 6 12 34 56 78\n33612345678\nFirst linecontinued."
    );

    let event = index.item(Some("home"), 6).unwrap().unwrap().extract;
    assert_eq!(
        (event.title.as_str(), event.place.as_str()),
        ("Weekly sync", "Room 2")
    );
}

#[test]
fn every_chunk_vector_derives() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let vectors = json(&spec.join("vectors/search/chunks.json"));

    for case in vectors["cases"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let mut got = Vec::new();
        for field in ["body", "note"] {
            let Some(parts) = case["fields"].get(field) else {
                continue;
            };
            let text: String = parts
                .as_array()
                .unwrap()
                .iter()
                .map(|part| match part.as_str() {
                    Some(text) => text.to_string(),
                    None => part["repeat"]
                        .as_str()
                        .unwrap()
                        .repeat(part["times"].as_u64().unwrap() as usize),
                })
                .collect();
            for range in chunk::chunks(&text) {
                got.push(serde_json::json!({
                    "ord": got.len(),
                    "field": field,
                    "start": range.start,
                    "end": range.end,
                }));
            }
        }
        assert_eq!(Value::from(got), case["chunks"], "{label}");
    }
}

/// A message with one subject and one body line.
fn message(id: &str, subject: &str, text: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}>\r\nSubject: {subject}\r\nFrom: Ann <ann@example.org>\r\n\
         Date: Sat, 01 Aug 2026 12:00:00 +0000\r\n\r\n{text}\r\n"
    )
    .into_bytes()
}

/// Files a message into a collection as source `local`.
fn file_message(dir: &Path, collection: &str, body: Vec<u8>, flags: &[&str]) {
    let mut store = PimdirStore::open(dir).unwrap().for_source("local");
    store
        .ensure_collection(collection, "message/rfc822")
        .unwrap();
    let derived = summary::derive("message/rfc822", &body).unwrap();
    let hash = store.hash(&body);
    store
        .write(vec![
            PimdirWriteOp::StoreObject {
                object: PimdirObject {
                    hash: hash.clone(),
                    size: body.len(),
                },
                body: Some(body),
            },
            PimdirWriteOp::UpsertPlacement(placement(
                collection,
                derived.link_id.as_str(),
                derived.link_id.as_str(),
                Some(hash),
                derived.summary.unwrap(),
                derived.sort_key.as_str(),
                flags.iter().copied().collect(),
            )),
        ])
        .unwrap();
}

fn hits(index: &PimdirIndex, query: &str) -> Vec<i64> {
    index
        .search(
            &PimdirQuery::parse(query).unwrap(),
            &PimdirSearchOptions::default(),
        )
        .unwrap()
        .hits
        .iter()
        .map(|hit| hit.seq)
        .collect()
}

#[test]
fn a_refresh_follows_the_feed_within_its_deadline() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );
    file_message(
        dir.path(),
        "Archive",
        message("b@x", "Beta", "second body"),
        &[],
    );

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    let past = PimdirIndexRefreshOptions {
        batch: 1,
        deadline: Some(std::time::Instant::now()),
        now: None,
    };
    let report = index.refresh(&past).unwrap();
    assert_eq!(
        (report.folded, report.cursor),
        (0, 0),
        "nothing past the deadline"
    );
    let stale = index.count(&PimdirQuery::parse("body").unwrap()).unwrap();
    assert_eq!(stale.count, 0);
    assert_eq!(
        stale.coverage.local.unindexed, 2,
        "the answer says what it missed"
    );
    assert!(!stale.coverage.is_complete());

    let report = index
        .refresh(&PimdirIndexRefreshOptions {
            batch: 1,
            ..Default::default()
        })
        .unwrap();
    assert!(report.is_current());
    assert_eq!((report.folded, report.indexed), (2, 2));
    assert_eq!(hits(&index, "body").len(), 2);
    assert!(
        index
            .count(&PimdirQuery::parse("body").unwrap())
            .unwrap()
            .coverage
            .is_complete()
    );

    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &["\\Flagged"],
    );
    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert_eq!(
        (report.folded, report.indexed),
        (1, 0),
        "a body is indexed once"
    );
    assert_eq!(hits(&index, "is:flagged").len(), 1);

    PimdirStore::open(dir.path())
        .unwrap()
        .delete_collection("Archive")
        .unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert!(
        report.reconciled,
        "a purge moves the count the index follows"
    );
    assert_eq!(report.removed, 1);
    assert_eq!(hits(&index, "beta"), Vec::<i64>::new());
}

#[test]
fn an_index_built_otherwise_is_rebuilt_and_a_held_lock_skips_the_refresh() {
    let dir = tempfile::tempdir().unwrap();
    file_message(
        dir.path(),
        "INBOX",
        message("a@x", "Alpha", "first body"),
        &[],
    );

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    drop(index);

    let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
    conn.execute("UPDATE index_meta SET tokenizer = 'porter'", [])
        .unwrap();
    drop(conn);

    let mut index = PimdirIndex::open(dir.path()).unwrap();
    assert_eq!(index.cursor().unwrap(), 0, "rebuilt, not migrated");

    let held = fs::File::create(dir.path().join("index.lock")).unwrap();
    fs4::FileExt::try_lock(&held).unwrap();
    let report = index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();
    assert!(report.busy && !report.is_current());
    drop(held);

    assert!(
        index
            .refresh(&PimdirIndexRefreshOptions::default())
            .unwrap()
            .is_current()
    );
    assert_eq!(hits(&index, "alpha").len(), 1);
}

/// The bodiless live items per kind a query's coverage counts, from the
/// vector store: those no file collection holds (SEARCH §8.2), in the
/// collections its top-level `account:`, `collection:` and `kind:` terms
/// narrow to.
fn bodiless(store: &Value, query: &PimdirQuery) -> BTreeMap<String, u64> {
    let conjuncts = match query {
        PimdirQuery::And(operands) => operands.iter().collect(),
        query => vec![query],
    };
    let collections = store["collections"].as_array().unwrap();
    let in_scope = |collection: &Value| {
        conjuncts.iter().all(|conjunct| {
            let PimdirQuery::Term(term) = conjunct else {
                return true;
            };
            match &term.predicate {
                PimdirQueryPredicate::Account(account) => collection["account"] == *account,
                PimdirQueryPredicate::Collection(id) => collection["id"] == *id,
                PimdirQueryPredicate::Kind(kind) => {
                    let media = match kind.as_str() {
                        "mail" => "message/rfc822",
                        "contact" => "text/vcard",
                        "file" => "application/octet-stream",
                        _ => "text/calendar",
                    };
                    collection["kind"] == media
                }
                _ => true,
            }
        })
    };

    let mut counts = BTreeMap::new();
    for item in store["items"].as_array().unwrap() {
        let collection = collections
            .iter()
            .find(|c| c["id"] == item["collection"])
            .unwrap();
        let kind = collection["kind"].as_str().unwrap();
        if item.get("fixture").is_none()
            && kind != "application/octet-stream"
            && in_scope(collection)
        {
            *counts.entry(kind.to_string()).or_default() += 1;
        }
    }
    counts
}

/// An RFC 3339 instant in UTC, `YYYY-MM-DDTHH:MM:SSZ`, as Unix seconds.
fn unix(instant: &str) -> i64 {
    let part = |range: core::ops::Range<usize>| instant[range].parse::<i64>().unwrap();
    let (year, month, day) = (part(0..4), part(5..7), part(8..10));

    // NOTE: days from the epoch to a civil date, proleptic Gregorian.
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;

    days * 86_400 + part(11..13) * 3_600 + part(14..16) * 60 + part(17..19)
}

/// Every text field alone answers in every order: a term matching bodies
/// alone once ranked outside its full-text query, which FTS5 refuses.
#[test]
fn every_field_alone_answers_in_every_order() {
    let Some(spec) = spec_dir() else {
        eprintln!("skipped: no pimdir spec checkout beside this one");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    build(&spec, dir.path());
    let mut index = PimdirIndex::open(dir.path()).unwrap();
    index
        .refresh(&PimdirIndexRefreshOptions::default())
        .unwrap();

    let queries = [
        "location:room",
        "location:\"Room 2\"",
        "attachment:invoice",
        "mimetype:pdf",
        "body:roadmap",
        "body:roa*",
        "title:sync",
        "subject:sync",
        "summary:sync",
        "with:alice",
        "from:alice",
        "to:bob",
        "cc:carol",
        "bcc:bob",
        "email:jane",
        "organizer:alice",
        "attendee:bob",
        "person:jane",
        "not body:roadmap",
    ];
    let sorts = [
        PimdirSearchSort::Date,
        PimdirSearchSort::Name,
        PimdirSearchSort::Relevance,
    ];
    for query in queries {
        let parsed = PimdirQuery::parse(query).unwrap();
        for sort in sorts {
            let options = PimdirSearchOptions {
                sort,
                ..Default::default()
            };
            if let Err(err) = index.search(&parsed, &options) {
                panic!("{query:?} sorted {sort:?}: {err}");
            }
        }
    }

    let room = index
        .search(
            &PimdirQuery::parse("location:room").unwrap(),
            &PimdirSearchOptions::default(),
        )
        .unwrap();
    assert_eq!(room.hits.len(), 1, "the event in Room 2");
}
