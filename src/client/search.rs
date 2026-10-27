//! # Search index
//!
//! index.db beside the store (SEARCH §2, §3): the full-text index over a
//! store's bodies and summaries, refreshed from its change feed and asked
//! in the query language of [`query`](crate::search::query).
//!
//! [`PimdirIndex`] is the indexer and the query client in one handle. It
//! opens index.db as its main database with pimdir.db attached read-only
//! as `store`, so one statement joins both, and rebuilds it rather than
//! migrating it when its version or tokenizer is not this build's. A
//! refresh takes index.lock exclusively, on the owner lock's terms: one
//! held elsewhere is reported, never waited out. The index is derived:
//! dropping index.db loses nothing.
//!
//! A one-shot answer refreshes first within a deadline and says in its
//! coverage what it could not fold in; a session following the feed calls
//! [`refresh`](PimdirIndex::refresh) whenever the store moves.

mod compile;
pub mod extract;

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use fs4::FileExt;
use log::{debug, trace};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, named_params,
    params_from_iter,
};

use crate::{
    client::{
        PimdirError, busy_or_sql,
        reader::PimdirReader,
        rows,
        search::{
            compile::Compiled,
            extract::{PimdirChunk, PimdirExtract, PimdirExtractStatus},
        },
        write::{load_addresses, load_summaries, tables_of},
    },
    codec,
    object::PimdirHash,
    placement::{PimdirFlags, PimdirLevel, PimdirLinkId},
    search::{
        coverage::PimdirSearchCoverage,
        query::{PimdirQuery, PimdirQueryKind, PimdirQueryPredicate},
    },
    sql::search,
    summary::file,
};

/// The file a refresh locks exclusively (SEARCH §3).
const LOCK: &str = "index.lock";

/// The SQLite version the index requires, for `contentless_delete`
/// (SEARCH §2), as `sqlite3_libversion_number` spells it.
const SQLITE_MIN: i32 = 3_043_000;

/// About how many characters a snippet holds.
const SNIPPET_LEN: usize = 200;

/// A store's search index: the indexer and the query client.
pub struct PimdirIndex {
    conn: Connection,
    reader: PimdirReader,
    dir: PathBuf,
}

/// How far one refresh goes.
#[derive(Clone, Copy, Debug)]
pub struct PimdirIndexRefreshOptions {
    /// The most feed rows one transaction folds, so the store's WAL still
    /// checkpoints between them (SEARCH §4).
    pub batch: usize,
    /// When to stop: checked between batches, so the pass ends at the
    /// first batch boundary past it with the cursor at that boundary.
    /// `None` folds the whole feed.
    pub deadline: Option<Instant>,
}

impl Default for PimdirIndexRefreshOptions {
    fn default() -> Self {
        Self {
            batch: 256,
            deadline: None,
        }
    }
}

/// What one refresh did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PimdirIndexReport {
    /// Feed rows folded.
    pub folded: u64,
    /// Bodies extracted and indexed for the first time.
    pub indexed: u64,
    /// Placements dropped: deleted, retained, purged or renamed away.
    pub removed: u64,
    /// Whether the key reconciliation ran (SEARCH §4).
    pub reconciled: bool,
    /// The store's last stamp when the pass began.
    pub head: i64,
    /// The index cursor the pass left: every stamp up to it is folded.
    pub cursor: i64,
    /// Whether another indexer held index.lock, so this one did nothing.
    pub busy: bool,
}

impl PimdirIndexReport {
    /// Whether the index has folded every change the store had when the
    /// pass began.
    pub fn is_current(&self) -> bool {
        !self.busy && self.cursor >= self.head
    }
}

/// The order of a hit list (SEARCH §8.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PimdirSearchSort {
    /// `sort_key` descending, contacts last.
    #[default]
    Date,
    /// The title ascending, case folded.
    Name,
    /// FTS5 `bm25`, best first; hits no text term ranks after, by date.
    Relevance,
}

/// One page of a search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PimdirSearchOptions {
    /// The order.
    pub sort: PimdirSearchSort,
    /// The most hits answered.
    pub limit: usize,
    /// The hits skipped before the first answered.
    pub offset: usize,
}

impl Default for PimdirSearchOptions {
    fn default() -> Self {
        Self {
            sort: PimdirSearchSort::Date,
            limit: 50,
            offset: 0,
        }
    }
}

/// A search's answer: one page of hits and what it could not see.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchOutput {
    /// The hits, in the order asked.
    pub hits: Vec<PimdirSearchHit>,
    /// Whether hits follow this page.
    pub more: bool,
    /// What the answer could not see (SEARCH §8.2).
    pub coverage: PimdirSearchCoverage,
}

/// A count's answer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchCountOutput {
    /// How many hits the query has.
    pub count: u64,
    /// What the count could not see (SEARCH §8.2).
    pub coverage: PimdirSearchCoverage,
}

/// One hit (SEARCH §8.2): an item in one account, addressed as
/// `(account, seq)`, with every live placement of it there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchHit {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The public id (STORAGE §9.1).
    pub seq: i64,
    /// The kind, as `kind:` names it, or the collection's media type for
    /// one the query language has no name for.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// The latest sort key of its placements.
    pub sort_key: String,
    /// Its live placements in the account, in collection order.
    pub placements: Vec<PimdirSearchPlacement>,
    /// What a text term matched: its body or its summary; `None` when only
    /// structured terms did.
    pub matched: Option<PimdirSearchMatched>,
    /// A passage of its body, re-read from the blob: from the first chunk
    /// naming a query word, else the first; `None` without an indexed
    /// body or a chunk.
    pub snippet: Option<String>,
}

/// One live placement of a hit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchPlacement {
    /// The collection.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// Its flags there.
    pub flags: PimdirFlags,
    /// The body's hash, `None` while bodiless.
    pub object: Option<PimdirHash>,
    /// The detail tier reached.
    pub level: PimdirLevel,
    /// Its sort key.
    pub sort_key: String,
}

/// What a text term matched (SEARCH §8.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PimdirSearchMatched {
    /// The indexed body.
    Body,
    /// The summary of a placement holding no body.
    Summary,
}

impl PimdirSearchMatched {
    /// The spelling the JSON output uses.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Summary => "summary",
        }
    }
}

/// An item as the index reads it: the hit's address and placements, and
/// its text by field as extraction gives it (SEARCH §6).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchItem {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// The kind, as [`PimdirSearchHit::kind`].
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// Its live placements in the account.
    pub placements: Vec<PimdirSearchPlacement>,
    /// What the text was read from: the body, or the summary of an item
    /// holding none.
    pub source: PimdirSearchMatched,
    /// The text by field.
    pub extract: PimdirExtract,
    /// The chunks of its `body` and `note` fields.
    pub chunks: Vec<PimdirChunk>,
}

/// One row of the change feed as `load_changed_items` answers it.
struct FeedRow {
    collection: String,
    link_id: String,
    seq: i64,
    object: Option<String>,
    flags: PimdirFlags,
    gone: bool,
    kind: String,
    /// The stamp the row took, which a batch records as the cursor it
    /// reached (SEARCH §4).
    changed: i64,
}

impl PimdirIndex {
    /// Opens the index of the store rooted at `dir`, creating index.db
    /// when absent and rebuilding it when its version or tokenizer is not
    /// this build's (SEARCH §2).
    ///
    /// The store must exist ([`PimdirError::Uncreated`]). A SQLite without
    /// FTS5 or older than 3.43 is [`PimdirError::Fts5`]; another indexer
    /// holding index.lock while this one has to build is
    /// [`PimdirError::Indexing`].
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PimdirError> {
        let dir = dir.as_ref();
        debug!("opening search index at {}", dir.display());

        let reader = PimdirReader::open(dir)?;
        if rusqlite::version_number() < SQLITE_MIN {
            return Err(PimdirError::Fts5 {
                version: rusqlite::version(),
            });
        }

        let mut conn = connect(dir)?;
        if !is_current(&conn)? {
            let Some(_lock) = lock(dir)? else {
                return Err(PimdirError::Indexing(dir.to_path_buf()));
            };
            // NOTE: another indexer may have built it between the check
            // and the lock.
            if !is_current(&conn)? {
                debug!("building search index at {}", dir.display());
                drop(conn);
                for name in ["index.db", "index.db-wal", "index.db-shm"] {
                    match fs::remove_file(dir.join(name)) {
                        Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.into()),
                        _ => (),
                    }
                }
                conn = connect(dir)?;
                init(&mut conn)?;
            }
        }

        Ok(Self {
            conn,
            reader,
            dir: dir.to_path_buf(),
        })
    }

    /// The store's reader this index reads summaries, coverage and bodies
    /// through.
    pub fn reader(&self) -> &PimdirReader {
        &self.reader
    }

    /// The store's last stamp this index has folded in (SEARCH §4).
    pub fn cursor(&self) -> Result<i64, PimdirError> {
        Ok(self
            .conn
            .query_row(search::LOAD_INDEX_META, [], |row| row.get(2))?)
    }

    /// Folds the store's change feed into the index (SEARCH §4), in
    /// transactions of at most `options.batch` rows, until the feed is
    /// exhausted or the deadline passes.
    ///
    /// Each transaction writes the cursor it reached, so a pass cut short
    /// resumes there; the one exhausting the feed also reconciles keys
    /// when the store's purge count or a collection stamp moved, meets
    /// again the bodies an earlier pass could not read, and writes the
    /// cursor read before the pass, so a crash replays and skips nothing.
    /// Another indexer holding index.lock makes this one return at once,
    /// [`busy`](PimdirIndexReport::busy).
    pub fn refresh(
        &mut self,
        options: &PimdirIndexRefreshOptions,
    ) -> Result<PimdirIndexReport, PimdirError> {
        let mut report = PimdirIndexReport {
            cursor: self.cursor()?,
            ..Default::default()
        };
        let head = self.reader.change_cursor()?;
        report.head = head.changed;

        let Some(_lock) = lock(&self.dir)? else {
            debug!("another indexer holds {LOCK}, skipping the refresh");
            report.busy = true;
            return Ok(report);
        };
        debug!(
            "refreshing search index from stamp {} to {}",
            report.cursor, head.changed
        );

        let (start, purges): (i64, i64) =
            self.conn
                .query_row(search::LOAD_INDEX_META, [], |r| Ok((r.get(2)?, r.get(3)?)))?;
        let batch = options.batch.max(1);
        let blobs = self.reader.blobs();

        loop {
            if options
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                debug!("refresh deadline passed at stamp {}", report.cursor);
                break;
            }

            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(busy_or_sql)?;
            let changed = feed(
                &tx,
                search::LOAD_CHANGED_ITEMS,
                named_params! { ":since": report.cursor, ":limit": batch as i64 },
            )?;
            let end = changed.last().map(|row| row.changed);
            trace!("folding {} feed rows", changed.len());

            for row in &changed {
                fold(&tx, &blobs, row, &mut report)?;
            }
            report.folded += changed.len() as u64;

            if changed.len() == batch {
                report.cursor = end.unwrap_or(report.cursor);
                tx.execute(
                    search::SET_INDEX_CURSOR,
                    named_params! { ":store_change": report.cursor, ":store_purges": purges },
                )?;
                tx.commit().map_err(busy_or_sql)?;
                continue;
            }

            let moved: bool = tx.query_row(
                search::COLLECTIONS_MOVED,
                named_params! { ":since": start },
                |r| r.get(0),
            )?;
            if head.purges != purges || moved {
                reconcile(&tx, &mut report)?;
            }

            let unread = feed(
                &tx,
                search::LOAD_UNREAD_BODIES,
                named_params! { ":limit": batch as i64 },
            )?;
            for row in &unread {
                fold(&tx, &blobs, row, &mut report)?;
            }

            report.cursor = end.unwrap_or(report.cursor).max(head.changed);
            tx.execute(
                search::SET_INDEX_CURSOR,
                named_params! { ":store_change": report.cursor, ":store_purges": head.purges },
            )?;
            tx.commit().map_err(busy_or_sql)?;
            break;
        }

        debug!(
            "refreshed search index: {} rows folded, {} bodies indexed, cursor {}",
            report.folded, report.indexed, report.cursor
        );
        Ok(report)
    }

    /// One page of the hits of a query (SEARCH §8.2), with what it could
    /// not see. Answers from the index as it stands: refresh first.
    pub fn search(
        &self,
        query: &PimdirQuery,
        options: &PimdirSearchOptions,
    ) -> Result<PimdirSearchOutput, PimdirError> {
        let mut compiled = Compiled::new(query)?;
        // NOTE: one more than asked, so the page knows whether it is the
        // last without a count.
        let probe = options.limit.saturating_add(1);
        let page = match options.sort {
            PimdirSearchSort::Name => None,
            _ => Some((probe, options.offset)),
        };
        let sql = compiled.select(options.sort, page);
        trace!("search statement: {sql}");

        let mut found = rows(
            &self.conn,
            &sql,
            params_from_iter(compiled.params.iter()),
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )?;
        if options.sort == PimdirSearchSort::Name {
            // NOTE: the title folded as STORAGE Annex A.2 folds `fn`,
            // ties to the seq then the account, as the SQL order gave.
            found.sort_by_cached_key(|(_, _, _, title)| title.to_lowercase().trim().to_string());
            found = found.into_iter().skip(options.offset).take(probe).collect();
        }
        let more = found.len() > options.limit;
        found.truncate(options.limit);

        let mut hits = Vec::new();
        for (account, seq, matched, _) in found {
            let (kind, title, placements) = self.placements(account.as_deref(), seq)?;
            let snippet = self.snippet(&placements, &compiled.words)?;
            hits.push(PimdirSearchHit {
                account,
                seq,
                kind,
                title,
                sort_key: placements
                    .iter()
                    .map(|placement| placement.sort_key.clone())
                    .max()
                    .unwrap_or_default(),
                placements,
                matched: match matched.as_deref() {
                    Some("body") => Some(PimdirSearchMatched::Body),
                    Some("summary") => Some(PimdirSearchMatched::Summary),
                    _ => None,
                },
                snippet,
            });
        }

        Ok(PimdirSearchOutput {
            hits,
            more,
            coverage: self.coverage(query)?,
        })
    }

    /// How many hits a query has, with what the count could not see.
    pub fn count(&self, query: &PimdirQuery) -> Result<PimdirSearchCountOutput, PimdirError> {
        let compiled = Compiled::new(query)?;
        let count: i64 = self.conn.query_row(
            &compiled.count(),
            params_from_iter(compiled.params.iter()),
            |row| row.get(0),
        )?;

        Ok(PimdirSearchCountOutput {
            count: count as u64,
            coverage: self.coverage(query)?,
        })
    }

    /// One item by the address a hit carries, read as the index reads it:
    /// its body extracted afresh, or its summary when it holds none.
    /// `None` when no live placement of it is in the account.
    pub fn item(
        &self,
        account: Option<&str>,
        seq: i64,
    ) -> Result<Option<PimdirSearchItem>, PimdirError> {
        let (kind, title, placements) = self.placements(account, seq)?;
        let Some(first) = placements.first() else {
            return Ok(None);
        };

        let body = placements
            .iter()
            .find_map(|placement| placement.object.as_ref());
        let blob = match body {
            Some(hash) if kind != PimdirQueryKind::File.as_str() => {
                self.reader.blobs().get(hash)?
            }
            _ => None,
        };
        let media_type = self
            .reader
            .collection_kind(&first.collection)?
            .unwrap_or_default();
        let (source, extract) = match blob {
            Some(blob) => (
                PimdirSearchMatched::Body,
                extract(
                    &self.conn,
                    &first.collection,
                    &first.link_id.0,
                    &media_type,
                    Some(&blob),
                )?,
            ),
            None => (
                PimdirSearchMatched::Summary,
                extract(
                    &self.conn,
                    &first.collection,
                    &first.link_id.0,
                    &media_type,
                    None,
                )?,
            ),
        };

        Ok(Some(PimdirSearchItem {
            account: account.map(ToString::to_string),
            seq,
            kind,
            title,
            placements,
            source,
            chunks: extract.chunks(),
            extract,
        }))
    }

    /// A hit's kind, title and live placements in the account.
    fn placements(
        &self,
        account: Option<&str>,
        seq: i64,
    ) -> Result<(String, String, Vec<PimdirSearchPlacement>), PimdirError> {
        let mut described = (String::new(), String::new());
        let placements = rows(
            &self.conn,
            search::LOAD_HIT_PLACEMENTS,
            named_params! { ":seq": seq, ":account": account },
            |row| {
                let kind: Option<String> = row.get(7)?;
                let media_type: String = row.get(6)?;
                Ok((
                    PimdirSearchPlacement {
                        collection: row.get(0)?,
                        link_id: PimdirLinkId(row.get(1)?),
                        flags: codec::flags_from_json(row.get::<_, Option<String>>(2)?.as_deref()),
                        object: row.get::<_, Option<String>>(3)?.map(PimdirHash),
                        sort_key: row.get(4)?,
                        level: codec::level_from_int(row.get(5)?),
                    },
                    kind.unwrap_or(media_type),
                    row.get::<_, String>(8)?,
                ))
            },
        )?;

        if let Some((_, kind, title)) = placements.first() {
            described = (kind.clone(), title.clone());
        }
        let placements = placements
            .into_iter()
            .map(|(placement, _, _)| placement)
            .collect();
        Ok((described.0, described.1, placements))
    }

    /// A passage of a hit's indexed body: the first chunk naming one of
    /// `words`, else the first chunk, trimmed to about [`SNIPPET_LEN`]
    /// characters around the word.
    fn snippet(
        &self,
        placements: &[PimdirSearchPlacement],
        words: &[String],
    ) -> Result<Option<String>, PimdirError> {
        let Some(hash) = placements
            .iter()
            .find_map(|placement| placement.object.as_ref())
        else {
            return Ok(None);
        };
        let chunks = rows(
            &self.conn,
            search::CHUNKS_OF,
            named_params! { ":hash": hash.0 },
            |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as usize,
                    row.get::<_, i64>(3)? as usize,
                ))
            },
        )?;
        if chunks.is_empty() {
            return Ok(None);
        }
        let Some(blob) = self.reader.blobs().get(hash)? else {
            return Ok(None);
        };

        let extract = PimdirExtract::mail(&blob);
        let words: Vec<String> = words.iter().map(|word| fold_text(word)).collect();
        let texts: Vec<&str> = chunks
            .iter()
            .filter_map(|(field, start, end)| extract.field(field)?.get(*start..*end))
            .collect();

        for text in &texts {
            let folded = fold_text(text);
            if let Some(at) = words
                .iter()
                .filter(|word| !word.is_empty())
                .filter_map(|word| folded.find(word.as_str()))
                .min()
            {
                let at = folded[..at].chars().count();
                return Ok(Some(trim(text, at)));
            }
        }
        Ok(texts.first().map(|text| trim(text, 0)))
    }

    /// What an answer to a query could not see: the coverage of the
    /// collections its top-level conjunction narrows to, over the date
    /// range it narrows to, and the live items there the index has not
    /// folded in yet.
    fn coverage(&self, query: &PimdirQuery) -> Result<PimdirSearchCoverage, PimdirError> {
        let conjuncts = match query {
            PimdirQuery::And(operands) => operands.iter().collect(),
            query => vec![query],
        };

        let mut collections = self.reader.list_collections()?;
        let mut since: Option<String> = None;
        let mut until: Option<String> = None;
        for conjunct in conjuncts {
            let PimdirQuery::Term(term) = conjunct else {
                continue;
            };
            match &term.predicate {
                PimdirQueryPredicate::Account(account) => {
                    collections.retain(|c| c.account.as_deref() == Some(account.as_str()))
                }
                PimdirQueryPredicate::Collection(id) => {
                    collections.retain(|c| &c.id == id || &c.name == id)
                }
                PimdirQueryPredicate::Kind(kind) => {
                    collections.retain(|c| c.kind == compile::media_type(*kind))
                }
                PimdirQueryPredicate::Date(range) => {
                    let (start, end) = compile::bounds(range);
                    if start > since {
                        since = start;
                    }
                    if let Some(end) = end
                        && until.as_ref().is_none_or(|until| &end < until)
                    {
                        until = Some(end);
                    }
                }
                _ => (),
            }
        }

        let ids: Vec<&str> = collections.iter().map(|c| c.id.as_str()).collect();
        let mut coverage = self
            .reader
            .search_coverage(&ids, since.as_deref(), until.as_deref())?;

        let unindexed: i64 = self.conn.query_row(
            search::COUNT_UNINDEXED,
            named_params! {
                ":since": self.cursor()?,
                ":collections": serde_json::to_string(&ids)?,
            },
            |r| r.get(0),
        )?;
        coverage.local.unindexed = unindexed as u64;
        Ok(coverage)
    }
}

/// Opens index.db as the main database, the store attached read-only as
/// `store` through a URI (SEARCH §3).
fn connect(dir: &Path) -> Result<Connection, PimdirError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(dir.join("index.db"), flags)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 30000;",
    )?;

    let store = dir.join("pimdir.db").canonicalize()?;
    conn.execute(
        "ATTACH DATABASE ?1 AS store",
        [format!("file:{}?mode=ro", uri_path(&store))],
    )?;
    Ok(conn)
}

/// Whether index.db holds this build's schema under this build's
/// tokenizer.
fn is_current(conn: &Connection) -> Result<bool, PimdirError> {
    let version: i64 = conn.pragma_query_value(Some("main"), "user_version", |r| r.get(0))?;
    if version != search::VERSION {
        return Ok(false);
    }

    let meta: Option<(i64, String)> = conn
        .query_row(search::LOAD_INDEX_META, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(meta.is_some_and(|(version, tokenizer)| {
        version == search::VERSION && tokenizer == search::TOKENIZER
    }))
}

/// Applies the index schema to an empty index.db, refusing a SQLite that
/// lacks FTS5 as [`PimdirError::Fts5`].
fn init(conn: &mut Connection) -> Result<(), PimdirError> {
    let tx = conn.transaction()?;
    for migration in search::MIGRATIONS {
        if let Err(err) = tx.execute_batch(migration) {
            if err.to_string().contains("fts5") {
                return Err(PimdirError::Fts5 {
                    version: rusqlite::version(),
                });
            }
            return Err(err.into());
        }
    }
    tx.pragma_update(Some("main"), "user_version", search::VERSION)?;
    tx.execute(
        search::INIT_INDEX_META,
        named_params! { ":version": search::VERSION, ":tokenizer": search::TOKENIZER },
    )?;
    tx.commit()?;
    Ok(())
}

/// Takes index.lock exclusively, `None` when another indexer holds it.
fn lock(dir: &Path) -> Result<Option<File>, PimdirError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK))?;
    // NOTE: named through the trait, as the owner lock does: std's own
    // inherent method from 1.89 would shadow it.
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(Some(file)),
        Err(_) => Ok(None),
    }
}

/// A path as a `file:` URI path: every byte but the unreserved ones and
/// `/` percent-encoded.
fn uri_path(path: &Path) -> String {
    let mut out = String::new();
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            byte => out += &format!("%{byte:02X}"),
        }
    }
    out
}

/// Reads feed rows through a statement answering `load_changed_items`'
/// columns.
fn feed(
    tx: &Transaction,
    statement: &str,
    params: &[(&str, &dyn rusqlite::ToSql)],
) -> Result<Vec<FeedRow>, PimdirError> {
    Ok(rows(tx, statement, params, |r| {
        Ok(FeedRow {
            collection: r.get(0)?,
            link_id: r.get(1)?,
            seq: r.get(2)?,
            object: r.get(3)?,
            flags: codec::flags_from_json(r.get::<_, Option<String>>(4)?.as_deref()),
            gone: r.get::<_, i64>(5)? != 0 || r.get::<_, Option<String>>(6)?.is_some(),
            kind: r.get(7)?,
            changed: r.get(9)?,
        })
    })?)
}

/// Folds one feed row (SEARCH §4): a gone row loses its placement, a live
/// one has its body indexed when new, its placement upserted, its summary
/// text written while it has no body, and its flags replaced.
fn fold(
    tx: &Transaction,
    blobs: &crate::client::blobs::PimdirBlobs,
    row: &FeedRow,
    report: &mut PimdirIndexReport,
) -> Result<(), PimdirError> {
    let key = named_params! { ":collection": row.collection, ":seq": row.seq };

    if row.gone {
        remove(tx, &row.collection, row.seq, report)?;
        return Ok(());
    }

    let hash = match &row.object {
        Some(hash) if row.kind != file::KIND => index_object(tx, blobs, row, hash, report)?,
        _ => None,
    };

    let id: i64 = tx.query_row(
        search::UPSERT_PLACEMENT,
        named_params! {
            ":collection": row.collection,
            ":seq": row.seq,
            ":link_id": row.link_id,
            ":hash": hash,
        },
        |r| r.get(0),
    )?;
    tx.execute(search::DELETE_SUMMARY_TEXT, named_params! { ":rowid": id })?;
    if hash.is_none() {
        let text = extract(tx, &row.collection, &row.link_id, &row.kind, None)?;
        tx.execute(
            search::INSERT_SUMMARY_TEXT,
            named_params! {
                ":rowid": id,
                ":title": text.title,
                ":people": text.people,
                ":from": text.from,
                ":to": text.to,
                ":cc": text.cc,
                ":bcc": text.bcc,
                ":email": text.email,
                ":organizer": text.organizer,
                ":attendee": text.attendee,
            },
        )?;
    }

    tx.execute(search::REPLACE_FLAGS, key)?;
    for flag in row.flags.known().into_iter().flatten() {
        tx.execute(
            search::INSERT_FLAG,
            named_params! { ":flag": flag, ":collection": row.collection, ":seq": row.seq },
        )?;
    }

    Ok(())
}

/// Drops a placement, its flags and occurrences by cascade and its
/// summary text by the rowid the delete returns (SEARCH §4).
fn remove(
    tx: &Transaction,
    collection: &str,
    seq: i64,
    report: &mut PimdirIndexReport,
) -> Result<(), PimdirError> {
    let id: Option<i64> = tx
        .query_row(
            search::DELETE_PLACEMENT,
            named_params! { ":collection": collection, ":seq": seq },
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        tx.execute(search::DELETE_SUMMARY_TEXT, named_params! { ":rowid": id })?;
        report.removed += 1;
    }
    Ok(())
}

/// Indexes a body once per hash (SEARCH §6), answering the hash when it
/// is indexed, `None` when the blob directory no longer holds it: no
/// object row is written, and the row is met again on the next pass.
fn index_object(
    tx: &Transaction,
    blobs: &crate::client::blobs::PimdirBlobs,
    row: &FeedRow,
    hash: &str,
    report: &mut PimdirIndexReport,
) -> Result<Option<String>, PimdirError> {
    let indexed = tx
        .query_row(
            search::OBJECT_INDEXED,
            named_params! { ":hash": hash },
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if indexed.is_some() {
        return Ok(Some(hash.into()));
    }

    let Some(body) = blobs.get(&PimdirHash(hash.into()))? else {
        debug!("blob {hash} is missing, indexing {} later", row.seq);
        return Ok(None);
    };
    let text = extract(tx, &row.collection, &row.link_id, &row.kind, Some(&body))?;

    let id: i64 = tx.query_row(
        search::INSERT_OBJECT,
        named_params! { ":hash": hash, ":status": text.status.as_str() },
        |r| r.get(0),
    )?;
    tx.execute(
        search::INSERT_OBJECT_TEXT,
        named_params! {
            ":rowid": id,
            ":title": text.title,
            ":people": text.people,
            ":body": text.body,
            ":attachment": text.attachment,
            ":place": text.place,
            ":org": text.org,
            ":note": text.note,
            ":from": text.from,
            ":to": text.to,
            ":cc": text.cc,
            ":bcc": text.bcc,
            ":email": text.email,
            ":organizer": text.organizer,
            ":attendee": text.attendee,
        },
    )?;
    for (ord, chunk) in text.chunks().into_iter().enumerate() {
        tx.execute(
            search::INSERT_CHUNK,
            named_params! {
                ":object": id,
                ":ord": ord as i64,
                ":field": chunk.field,
                ":start": chunk.range.start as i64,
                ":end": chunk.range.end as i64,
            },
        )?;
    }

    report.indexed += 1;
    Ok(Some(hash.into()))
}

/// Extracts an item's text (SEARCH §6): a message's body through the MIME
/// walk, an unparseable one from what its summary could read; every other
/// kind, and an item holding no body, from its summary and addresses.
fn extract(
    conn: &Connection,
    collection: &str,
    link_id: &str,
    kind: &str,
    body: Option<&[u8]>,
) -> Result<PimdirExtract, PimdirError> {
    if let Some(body) = body
        && kind == "message/rfc822"
    {
        let extract = PimdirExtract::mail(body);
        if extract.status != PimdirExtractStatus::Unparseable {
            return Ok(extract);
        }
    }

    let links = serde_json::to_string(&[link_id])?;
    let mut summary = None;
    for table in tables_of(kind) {
        if let Some((_, found)) = load_summaries(conn, table, collection, Some(&links))?
            .into_iter()
            .next()
        {
            summary = Some(found);
            break;
        }
    }
    let addresses: Vec<_> = load_addresses(conn, collection, Some(&links))?
        .into_iter()
        .map(|(_, role, address)| (role, address))
        .collect();

    let mut extract = PimdirExtract::summary(summary.as_ref(), &addresses);
    if body.is_some() && kind == "message/rfc822" {
        extract.status = PimdirExtractStatus::Unparseable;
    }
    Ok(extract)
}

/// Drops the placements and the bodies the store no longer holds (SEARCH
/// §4): purged rows, a renamed collection's old id, collected objects.
fn reconcile(tx: &Transaction, report: &mut PimdirIndexReport) -> Result<(), PimdirError> {
    debug!("reconciling search index keys");
    report.reconciled = true;

    let gone = rows(tx, search::PLACEMENTS_GONE, [], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for (collection, seq) in gone {
        remove(tx, &collection, seq, report)?;
    }

    let objects = rows(tx, search::OBJECTS_GONE, [], |r| r.get::<_, String>(0))?;
    for hash in objects {
        let id: Option<i64> = tx
            .query_row(
                search::OBJECT_INDEXED,
                named_params! { ":hash": hash },
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            tx.execute(search::DELETE_OBJECT, named_params! { ":rowid": id })?;
        }
        tx.execute(search::DELETE_OBJECT_ROW, named_params! { ":hash": hash })?;
    }
    Ok(())
}

/// A text folded for a snippet's word search: lowercased, the common
/// Latin diacritics stripped, one character for one so offsets in
/// characters agree with the text's.
fn fold_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            let lower = c.to_lowercase().next().unwrap_or(c);
            match lower {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
                'ç' => 'c',
                'è' | 'é' | 'ê' | 'ë' => 'e',
                'ì' | 'í' | 'î' | 'ï' => 'i',
                'ñ' => 'n',
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
                'ù' | 'ú' | 'û' | 'ü' => 'u',
                'ý' | 'ÿ' => 'y',
                c => c,
            }
        })
        .collect()
}

/// About [`SNIPPET_LEN`] characters of a chunk around the character at
/// `at`, whitespace runs collapsed, an ellipsis marking each cut.
fn trim(text: &str, at: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(SNIPPET_LEN / 4);
    let start = match chars[..start].iter().rposition(|c| c.is_whitespace()) {
        Some(space) if start - space < 20 => space + 1,
        _ => start,
    };
    let end = (start + SNIPPET_LEN).min(chars.len());

    let mut snippet = String::new();
    if start > 0 {
        snippet.push('…');
    }
    let mut space = false;
    for c in &chars[start..end] {
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !snippet.is_empty() && !snippet.ends_with('…') {
            snippet.push(' ');
        }
        space = false;
        snippet.push(*c);
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}
