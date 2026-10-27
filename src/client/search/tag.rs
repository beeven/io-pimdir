//! # Tags
//!
//! Tagging from a query (SEARCH §10): the hits of a query, every live
//! placement of each, and the flag set each would carry once `add` is
//! added and `remove` removed, staged as one `set-flags` action per
//! placement through the queue (STORAGE §15), as a producer. A placement
//! the queue holds a `set-flags` of is planned from the flags that one
//! sets, so tags staged in a row compose.
//!
//! [`PimdirIndex::tag`] answers the plan without writing, so a caller can
//! show it before it writes; [`PimdirSearchTagPlan::stage`] appends it.
//! Applying it is the owner's: a drain, or the next sync.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use log::{debug, trace};

use crate::{
    capability::{self, PimdirSourceCapabilities, PimdirSupport},
    client::{
        PimdirError,
        producer::{PimdirPendingAction, PimdirProducer},
        search::{Found, PimdirIndex, PimdirSearchClock, PimdirSearchSort, compile::Compiled},
    },
    codec::PimdirAction,
    placement::{PimdirFlags, PimdirLinkId},
    search::{coverage::PimdirSearchCoverage, query::PimdirQuery},
};

/// What a tag stages (SEARCH §10): every hit of its query, each placement
/// with the flag set it would carry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PimdirSearchTagPlan {
    /// The flags added, `is:` spellings mapped to their flags.
    pub add: BTreeSet<String>,
    /// The flags removed, mapped likewise.
    pub remove: BTreeSet<String>,
    /// Every hit of the query, in date order.
    pub hits: Vec<PimdirSearchTagHit>,
    /// What the query could not see (SEARCH §8.2): a tag reaches only the
    /// hits the index answers.
    pub coverage: PimdirSearchCoverage,
}

/// One hit a tag reaches, `(account, seq)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchTagHit {
    /// The account, `None` in a single-account store.
    pub account: Option<String>,
    /// The public id.
    pub seq: i64,
    /// The kind, as a search hit names it.
    pub kind: String,
    /// Its subject, name or summary.
    pub title: String,
    /// Its live placements in the account.
    pub placements: Vec<PimdirSearchTagPlacement>,
}

/// One placement of a hit and the flag set a tag gives it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchTagPlacement {
    /// The collection.
    pub collection: String,
    /// The item's key.
    pub link_id: PimdirLinkId,
    /// Its flags now, as the last pending `set-flags` of it leaves them
    /// when the queue holds one.
    pub flags: PimdirFlags,
    /// The flag set the tag stages for it; `None` when its flags were
    /// never read, so no absolute set can be staged without losing them.
    pub next: Option<BTreeSet<String>>,
    /// The source its action is staged as, or why there is none.
    pub source: PimdirSearchTagSource,
    /// Whether its source carries the change to its server by its
    /// declared capabilities (Annex B.1): only mail has flags there, and
    /// only a source declaring the flags changed (or declaring nothing)
    /// pushes them. False keeps the tag in the store alone.
    pub syncs: bool,
    /// The capabilities the change needs that its source declares no
    /// support for (Annex B.1): not empty, the placement is not staged,
    /// since the source would refuse it.
    pub missing: Vec<String>,
}

/// The source a placement's action is staged as (STORAGE §15): one that
/// syncs its collection, never another, so it is pushed where the
/// collection lives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirSearchTagSource {
    /// The source asked for, which syncs the collection, else the only
    /// one that does.
    Resolved(String),
    /// Several sources sync the collection and none was asked for.
    Ambiguous(Vec<String>),
    /// The source asked for does not sync the collection, or no source
    /// does: the sources that do, possibly none.
    Unsynced(Vec<String>),
}

/// What a tag reads of one collection: the source its actions are staged
/// as, its kind, and that source's declaration there.
struct Collection {
    source: PimdirSearchTagSource,
    kind: String,
    declared: Vec<PimdirSourceCapabilities>,
}

impl PimdirSearchTagPlacement {
    /// Whether the tag changes its flags, so an action is staged for it
    /// when its source is resolved and supports the change.
    pub fn changes(&self) -> bool {
        self.next
            .as_ref()
            .is_some_and(|next| Some(next) != self.flags.known())
    }
}

/// One `set-flags` action a tag appended to the queue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PimdirSearchTagAction {
    /// The collection it is anchored on.
    pub collection: String,
    /// The item's public id.
    pub seq: i64,
    /// The source it is staged as: the one a drain applies it as.
    pub source: String,
    /// The flag set it sets.
    pub flags: BTreeSet<String>,
    /// The queue row's id.
    pub action: i64,
}

/// A flag change a tag refuses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PimdirSearchTagError {
    /// Nothing to add or remove.
    Empty,
    /// One flag both added and removed.
    Both(String),
    /// A value that names no flag: empty, or an `is:` value that is the
    /// absence of a flag (`unread`) or no flag at all (`encrypted`).
    NotAFlag(String),
}

impl fmt::Display for PimdirSearchTagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "A tag needs a flag to add or to remove"),
            Self::Both(flag) => write!(f, "Flag {flag} is both added and removed"),
            Self::NotAFlag(value) if value == "unread" => {
                write!(f, "unread is the absence of \\Seen: remove read instead")
            }
            Self::NotAFlag(value) => write!(f, "{value:?} names no flag"),
        }
    }
}

impl core::error::Error for PimdirSearchTagError {}

impl PimdirIndex {
    /// The plan of tagging every hit of a query (SEARCH §10): `add` added
    /// to and `remove` removed from each live placement's flags, the
    /// `is:` spellings `read`, `flagged`, `answered` and `draft` read as
    /// `\Seen`, `\Flagged`, `\Answered` and `\Draft` and any other value
    /// as written. Writes nothing; answers from the index as it stands.
    ///
    /// Each placement's action is staged as a source syncing its
    /// collection: `source` when it does, else the only one; several and
    /// no `source`, or a `source` not among them, leave the placement
    /// unstaged ([`PimdirSearchTagSource`]).
    ///
    /// An empty change, a flag both added and removed, and a value naming
    /// no flag are [`PimdirError::Tag`].
    pub fn tag(
        &self,
        query: &PimdirQuery,
        add: &[String],
        remove: &[String],
        source: Option<&str>,
        clock: &PimdirSearchClock,
    ) -> Result<PimdirSearchTagPlan, PimdirError> {
        let add = flags(add)?;
        let remove = flags(remove)?;
        if add.is_empty() && remove.is_empty() {
            return Err(PimdirSearchTagError::Empty.into());
        }
        if let Some(flag) = add.intersection(&remove).next() {
            return Err(PimdirSearchTagError::Both(flag.clone()).into());
        }
        debug!("planning a tag adding {add:?} and removing {remove:?}");

        let now = clock.now();
        let mut compiled = Compiled::new(query, self.ends(query)?, now);
        let found = self.found(&mut compiled, PimdirSearchSort::Date, None)?;

        let mut pending: BTreeMap<String, Vec<PimdirPendingAction>> = BTreeMap::new();
        let mut sources: BTreeMap<String, Collection> = BTreeMap::new();
        let mut hits = Vec::new();
        for Found { account, seq, .. } in found {
            let (kind, title, held) = self.placements(account.as_deref(), seq)?;
            let mut placements = Vec::new();
            for mut placement in held {
                if !pending.contains_key(&placement.collection) {
                    let actions = self.reader.pending_actions(&placement.collection)?;
                    pending.insert(placement.collection.clone(), actions);
                    let syncing = self.reader.collection_sources(&placement.collection)?;
                    let resolved = match (source, syncing.as_slice()) {
                        (Some(asked), syncing) if syncing.iter().any(|s| s == asked) => {
                            PimdirSearchTagSource::Resolved(asked.to_string())
                        }
                        (None, [only]) => PimdirSearchTagSource::Resolved(only.clone()),
                        (None, [_, _, ..]) => PimdirSearchTagSource::Ambiguous(syncing),
                        _ => PimdirSearchTagSource::Unsynced(syncing),
                    };
                    trace!("tag stages {} as {resolved:?}", placement.collection);
                    let kind = self
                        .reader
                        .collection_kind(&placement.collection)?
                        .unwrap_or_default();
                    let declared = match &resolved {
                        PimdirSearchTagSource::Resolved(source) => self
                            .reader
                            .capabilities(&placement.collection)?
                            .into_iter()
                            .filter(|declared| &declared.source == source)
                            .collect(),
                        _ => Vec::new(),
                    };
                    let collection = Collection {
                        source: resolved,
                        kind,
                        declared,
                    };
                    sources.insert(placement.collection.clone(), collection);
                }
                // NOTE: the flags a pending set-flags will set, so a tag
                // staged behind it builds on it rather than undoing it
                // (STORAGE §15.4).
                let staged = pending[&placement.collection]
                    .iter()
                    .rev()
                    .find_map(|pending| match &pending.action {
                        PimdirAction::SetFlags { seq: at, flags } if *at == seq => {
                            Some(flags.clone())
                        }
                        _ => None,
                    });
                if let Some(flags) = staged {
                    placement.flags = flags;
                }

                let next: Option<BTreeSet<String>> = placement.flags.known().map(|flags| {
                    flags
                        .union(&add)
                        .filter(|flag| !remove.contains(*flag))
                        .cloned()
                        .collect()
                });
                let collection = &sources[&placement.collection];
                // NOTE: flags exist on the wire for mail alone (Annex
                // B.1); an undeclared source is not gated, so it is taken
                // at its word as the gate takes it.
                let (syncs, missing) = match (&collection.source, &next) {
                    (PimdirSearchTagSource::Resolved(_), Some(next))
                        if collection.kind == "message/rfc822" =>
                    {
                        let action = PimdirAction::SetFlags {
                            seq,
                            flags: next.iter().collect(),
                        };
                        let needed =
                            capability::required(&collection.kind, &action, &placement.flags);
                        let missing: Vec<String> = needed
                            .into_iter()
                            .filter(|needed| {
                                collection.declared.iter().any(|source| {
                                    source.declared.as_ref().is_some_and(|declared| {
                                        !matches!(
                                            declared.get(needed),
                                            Some((PimdirSupport::Full | PimdirSupport::Partial, _))
                                        )
                                    })
                                })
                            })
                            .collect();
                        (missing.is_empty(), missing)
                    }
                    _ => (false, Vec::new()),
                };
                let source = collection.source.clone();
                placements.push(PimdirSearchTagPlacement {
                    collection: placement.collection,
                    link_id: placement.link_id,
                    flags: placement.flags,
                    next,
                    source,
                    syncs,
                    missing,
                });
            }
            hits.push(PimdirSearchTagHit {
                account,
                seq,
                kind,
                title,
                placements,
            });
        }
        trace!("tag reaches {} hit(s)", hits.len());

        Ok(PimdirSearchTagPlan {
            add,
            remove,
            hits,
            coverage: self.coverage(query, now)?,
        })
    }
}

impl PimdirSearchTagPlan {
    /// Appends one `set-flags` action per placement the plan changes,
    /// resolves a source for and finds no capability `missing` for,
    /// through `producer` (STORAGE §15), answering them in plan order.
    ///
    /// Every action passes the capability gate before any is appended, so
    /// one a source refuses ([`PimdirError::Unsupported`]), a declaration
    /// changed since the plan, leaves the queue as it was. A placement whose flags were never read is left
    /// out, as is one the change leaves as it is.
    pub fn stage(
        &self,
        producer: &mut PimdirProducer,
    ) -> Result<Vec<PimdirSearchTagAction>, PimdirError> {
        let mut staged = Vec::new();
        for hit in &self.hits {
            for placement in hit.placements.iter().filter(|p| p.changes()) {
                let PimdirSearchTagSource::Resolved(source) = &placement.source else {
                    continue;
                };
                if !placement.missing.is_empty() {
                    continue;
                }
                let flags = placement.next.clone().unwrap_or_default();
                let action = PimdirAction::SetFlags {
                    seq: hit.seq,
                    flags: flags.iter().collect(),
                };
                producer.check(&placement.collection, &action)?;
                staged.push((
                    placement.collection.clone(),
                    hit.seq,
                    source.clone(),
                    flags,
                    action,
                ));
            }
        }
        debug!("staging {} set-flags action(s)", staged.len());

        let mut actions = Vec::new();
        for (collection, seq, source, flags, action) in staged {
            let id = producer.enqueue(&collection, &action, None)?;
            trace!("staged action {id} on {collection} seq {seq}");
            actions.push(PimdirSearchTagAction {
                collection,
                seq,
                source,
                flags,
                action: id,
            });
        }
        Ok(actions)
    }
}

/// The flags of a tag's values (SEARCH §8): an `is:` spelling naming a
/// flag's presence read as that flag, any other value as written.
fn flags(values: &[String]) -> Result<BTreeSet<String>, PimdirSearchTagError> {
    values
        .iter()
        .map(|value| match value.as_str() {
            "read" => Ok("\\Seen".to_string()),
            "flagged" => Ok("\\Flagged".to_string()),
            "answered" => Ok("\\Answered".to_string()),
            "draft" => Ok("\\Draft".to_string()),
            "" | "unread" | "encrypted" => Err(PimdirSearchTagError::NotAFlag(value.clone())),
            flag => Ok(flag.to_string()),
        })
        .collect()
}
