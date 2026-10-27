//! # Compiler
//!
//! A [`PimdirQuery`] compiled to one SQL statement over index.db with the
//! store attached (SEARCH §8): every text term an FTS5 `MATCH` built from
//! escaped tokens, through the canonical `match_objects` and
//! `match_summaries`; every structured term a set of placements, through
//! the canonical statements where one exists. A hit is `(account, seq)`,
//! so each term is lifted to the hits any live placement of which it
//! holds for, and `not` is the hits none does: the union rule SEARCH §8.2
//! states for tags, kept for every term.

use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use rusqlite::types::Value;

use crate::{
    client::search::PimdirSearchSort,
    search::query::{
        PimdirQuery, PimdirQueryDate, PimdirQueryDateRange, PimdirQueryError, PimdirQueryErrorCode,
        PimdirQueryId, PimdirQueryKind, PimdirQueryPredicate, PimdirQueryTerm, PimdirQueryText,
        PimdirQueryTextField,
    },
    sql::{self, search},
    summary::file,
};

/// The items each `linked:` term names, as `(kind, link id)` pairs per
/// named id (STORAGE §14.2).
pub(crate) type Ends = Vec<(PimdirQueryId, Vec<(String, String)>)>;

/// A compiled query: its common table expressions, the condition a live
/// hit meets, the parameters both bind, and what an answer reports
/// beside its hits.
pub(crate) struct Compiled {
    /// One `t<n>(akey, seq, src, rank)` per text term.
    ctes: Vec<String>,
    /// The text terms no `not` negates, whose matches say what a hit
    /// matched on and rank it.
    positive: Vec<usize>,
    /// The condition over `live(akey, account, seq)`.
    condition: String,
    /// The parameters, bound by position.
    pub(crate) params: Vec<Value>,
    /// The words a snippet looks for: the positive text terms'.
    pub(crate) words: Vec<String>,
    /// The items each `linked:` term names, as `(kind, link id)` pairs
    /// (STORAGE §14.2), resolved against the store by the caller.
    ends: Ends,
}

impl Compiled {
    /// Compiles a query, `ends` holding the items each `linked:` term
    /// names, refusing a term this implementation does not answer yet.
    pub(crate) fn new(query: &PimdirQuery, ends: Ends) -> Result<Self, PimdirQueryError> {
        let undated: Vec<&str> = query
            .terms()
            .into_iter()
            .filter_map(|(term, _)| match &term.predicate {
                PimdirQueryPredicate::Kind(kind) if !kind.is_dated() => Some(media_type(*kind)),
                _ => None,
            })
            .collect();

        let mut compiled = Self {
            ctes: Vec::new(),
            positive: Vec::new(),
            condition: String::new(),
            params: Vec::new(),
            words: Vec::new(),
            ends,
        };
        compiled.condition = compiled.node(query, false, &undated)?;
        Ok(compiled)
    }

    /// The hits in `sort` order (SEARCH §8.2), `limit` of them after
    /// `offset`, every one with `None`: account, seq, what they matched
    /// on, rank, sort key, whether undated, title. The `name` order folds
    /// titles as SQL cannot, so its caller sorts the whole set itself.
    pub(crate) fn select(
        &mut self,
        sort: PimdirSearchSort,
        page: Option<(usize, usize)>,
    ) -> String {
        let order = match sort {
            PimdirSearchSort::Date => {
                "undated ASC, CASE WHEN undated = 0 THEN sort_key END DESC, \
                 CASE WHEN undated = 1 THEN sort_key END ASC, h.seq ASC, h.akey ASC"
            }
            PimdirSearchSort::Name => "h.seq ASC, h.akey ASC",
            PimdirSearchSort::Relevance => "rank ASC NULLS LAST, h.seq ASC, h.akey ASC",
        };
        let (limit, offset) = page.unwrap_or((i64::MAX as usize, 0));
        let limit = self.bind(Value::Integer(limit.min(i64::MAX as usize) as i64));
        let offset = self.bind(Value::Integer(offset as i64));
        let placements = "FROM store.items i JOIN store.collections c ON c.id = i.collection \
            WHERE i.seq = h.seq AND ifnull(c.account, '') = h.akey \
            AND i.deleted = 0 AND i.retained_at IS NULL";

        format!(
            "{}
SELECT h.account, h.seq,
  (SELECT CASE WHEN max(m.src = 'body') = 1 THEN 'body' WHEN count(*) > 0 THEN 'summary' END
   FROM matched m WHERE m.akey = h.akey AND m.seq = h.seq) AS matched,
  (SELECT min(m.rank) FROM matched m WHERE m.akey = h.akey AND m.seq = h.seq) AS rank,
  (SELECT max(i.sort_key) {placements}) AS sort_key,
  (SELECT max(c.kind IN ('text/vcard', 'application/octet-stream')) {placements}) AS undated,
  (SELECT coalesce(ms.subject, cs.fn, es.summary, ts.summary, js.summary, fs.name, '') {titles}
   WHERE i.seq = h.seq AND ifnull(c.account, '') = h.akey AND i.deleted = 0
     AND i.retained_at IS NULL ORDER BY i.collection LIMIT 1) AS title
FROM hits h ORDER BY {order} LIMIT {limit} OFFSET {offset};",
            self.with(),
            titles = TITLES,
        )
    }

    /// How many hits the query has.
    pub(crate) fn count(&self) -> String {
        format!("{}\nSELECT count(*) FROM hits;", self.with())
    }

    /// The common table expressions every statement starts with: the text
    /// terms, the live hits, the hits meeting the condition, and the
    /// matches of the positive text terms.
    fn with(&self) -> String {
        let mut ctes = self.ctes.clone();
        ctes.push(
            "live(akey, account, seq) AS (SELECT DISTINCT ifnull(c.account, ''), c.account, i.seq \
             FROM store.items i JOIN store.collections c ON c.id = i.collection \
             WHERE i.deleted = 0 AND i.retained_at IS NULL)"
                .into(),
        );
        ctes.push(format!(
            "hits(akey, account, seq) AS (SELECT akey, account, seq FROM live WHERE {})",
            self.condition
        ));

        let matched = match self.positive.is_empty() {
            true => "SELECT NULL, NULL, NULL, NULL WHERE 0".to_string(),
            false => self
                .positive
                .iter()
                .map(|at| format!("SELECT akey, seq, src, rank FROM t{at}"))
                .collect::<Vec<_>>()
                .join(" UNION ALL "),
        };
        ctes.push(format!("matched(akey, seq, src, rank) AS ({matched})"));

        format!("WITH {}", ctes.join(",\n"))
    }

    /// Binds one parameter, answering its position.
    fn bind(&mut self, value: Value) -> String {
        self.params.push(value);
        format!("?{}", self.params.len())
    }

    /// Embeds a canonical statement as a subquery: its comments and final
    /// semicolon stripped, each named parameter bound by position.
    fn embed(&mut self, statement: &str, binds: Vec<(&str, Value)>) -> String {
        let mut text: String = statement
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        text = text.trim().trim_end_matches(';').to_string();

        for (name, value) in binds {
            let position = self.bind(value);
            text = replace_param(&text, name, &position);
        }
        text
    }

    /// Compiles one node to a condition over `live`.
    fn node(
        &mut self,
        node: &PimdirQuery,
        negated: bool,
        undated: &[&str],
    ) -> Result<String, PimdirQueryError> {
        match node {
            PimdirQuery::And(operands) | PimdirQuery::Or(operands) => {
                let operator = match node {
                    PimdirQuery::And(_) => " AND ",
                    _ => " OR ",
                };
                let mut conditions = Vec::new();
                for operand in operands {
                    conditions.push(self.node(operand, negated, undated)?);
                }
                Ok(format!("({})", conditions.join(operator)))
            }
            PimdirQuery::Not(operand) => {
                Ok(format!("(NOT {})", self.node(operand, !negated, undated)?))
            }
            PimdirQuery::Term(term) => self.term(term, negated, undated),
        }
    }

    /// Compiles one term to a condition over `live`.
    fn term(
        &mut self,
        term: &PimdirQueryTerm,
        negated: bool,
        undated: &[&str],
    ) -> Result<String, PimdirQueryError> {
        let unsupported = |what: &str| {
            PimdirQueryError::new(
                PimdirQueryErrorCode::UnsupportedField,
                format!("{what} is not supported by this implementation yet"),
                term.start,
                term.end,
                &[],
            )
        };
        let live = "FROM store.items i JOIN store.collections c ON c.id = i.collection \
                    WHERE i.deleted = 0 AND i.retained_at IS NULL";

        let placements = match &term.predicate {
            PimdirQueryPredicate::Text { field, text } => {
                let (column, summary) = match field {
                    PimdirQueryTextField::Any => (None, true),
                    PimdirQueryTextField::Title => (Some("title"), true),
                    PimdirQueryTextField::Body => (Some("body"), false),
                    PimdirQueryTextField::Attachment => (Some("attachment"), false),
                    PimdirQueryTextField::Place => (Some("place"), false),
                };
                return Ok(self.text(column, summary, text, negated));
            }
            PimdirQueryPredicate::Address { role, text } if text.text.contains('@') => {
                let address = text.text.to_lowercase();
                let role = Value::from(role.map(|role| role.as_str().to_string()));
                match address.strip_prefix('@') {
                    Some(domain) => self.embed(
                        sql::LIST_DOMAIN_PLACEMENTS,
                        vec![(":domain", domain.to_string().into()), (":role", role)],
                    ),
                    None => self.embed(
                        sql::LIST_ADDRESS_PLACEMENTS,
                        vec![(":address", address.into()), (":role", role)],
                    ),
                }
            }
            PimdirQueryPredicate::Address { role, text } => {
                let column = role.map(|role| role.as_str()).unwrap_or("people");
                return Ok(self.text(Some(column), true, text, negated));
            }
            PimdirQueryPredicate::Person(text) => self.person(text),
            PimdirQueryPredicate::Thread(_) => return Err(unsupported("thread:")),
            PimdirQueryPredicate::Linked { role, id } => {
                let ends = self
                    .ends
                    .iter()
                    .find(|(named, _)| named == id)
                    .map(|(_, ends)| ends.clone())
                    .unwrap_or_default();
                let role = Value::from(role.as_ref().map(|role| role.as_str().to_string()));
                let mut linked = vec!["SELECT NULL AS collection, NULL AS seq WHERE 0".to_string()];
                for (kind, link_id) in ends {
                    linked.push(self.embed(
                        search::LINKED,
                        vec![
                            (":kind", kind.into()),
                            (":link_id", link_id.into()),
                            (":role", role.clone()),
                        ],
                    ));
                }
                linked.join(" UNION ")
            }
            PimdirQueryPredicate::Flag(flag) => {
                self.embed(search::FLAGGED, vec![(":flag", flag.clone().into())])
            }
            PimdirQueryPredicate::Encrypted => {
                "SELECT p.collection, p.seq FROM placement p JOIN object o ON o.hash = p.hash \
                 WHERE o.status = 'encrypted'"
                    .into()
            }
            PimdirQueryPredicate::HasBody => {
                format!("SELECT i.collection, i.seq {live} AND i.object_hash IS NOT NULL")
            }
            PimdirQueryPredicate::HasAttachment => format!(
                "SELECT i.collection, i.seq {live} AND EXISTS (SELECT 1 FROM store.mail_summary m \
                 WHERE m.collection = i.collection AND m.link_id = i.link_id AND m.attachment = 1)"
            ),
            PimdirQueryPredicate::Kind(kind) => match kind {
                PimdirQueryKind::Event | PimdirQueryKind::Task | PimdirQueryKind::Journal => {
                    let table = match kind {
                        PimdirQueryKind::Event => "event_summary",
                        PimdirQueryKind::Task => "task_summary",
                        _ => "journal_summary",
                    };
                    format!(
                        "SELECT i.collection, i.seq {live} AND EXISTS (SELECT 1 FROM store.{table} s \
                         WHERE s.collection = i.collection AND s.link_id = i.link_id)"
                    )
                }
                kind => {
                    let kind = self.bind(media_type(*kind).to_string().into());
                    format!("SELECT i.collection, i.seq {live} AND c.kind = {kind}")
                }
            },
            PimdirQueryPredicate::Account(account) => {
                return Ok(format!("(akey = {})", self.bind(account.clone().into())));
            }
            PimdirQueryPredicate::Collection(collection) => {
                let collection = self.bind(collection.clone().into());
                format!(
                    "SELECT i.collection, i.seq {live} \
                     AND (i.collection = {collection} OR c.name = {collection})"
                )
            }
            PimdirQueryPredicate::Date(range) => {
                if range
                    .start
                    .iter()
                    .chain(&range.end)
                    .any(|d| d.is_relative())
                {
                    return Err(unsupported("A relative date"));
                }
                let (since, until) = bounds(range);
                // NOTE: an open side is the earliest or the latest instant an
                // RFC 3339 string can spell.
                let occurring = self.embed(
                    search::OCCURRING_BETWEEN,
                    vec![
                        (":start", since.clone().unwrap_or_default().into()),
                        (
                            ":end",
                            until
                                .clone()
                                .unwrap_or_else(|| "9999-12-31T23:59:59Z".into())
                                .into(),
                        ),
                    ],
                );
                let since = self.bind(since.into());
                let until = self.bind(until.into());
                let undated = self.bind(serde_json::to_string(undated).unwrap_or_default().into());
                format!(
                    "SELECT i.collection, i.seq {live} AND ((c.kind = 'message/rfc822' \
                     OR (c.kind = 'text/calendar' AND i.object_hash IS NULL)) \
                     AND i.sort_key != '' AND ({since} IS NULL OR i.sort_key >= {since}) \
                     AND ({until} IS NULL OR i.sort_key < {until})) \
                     UNION SELECT collection, seq FROM ({occurring}) \
                     UNION SELECT i.collection, i.seq {live} \
                     AND c.kind IN (SELECT value FROM json_each({undated}))"
                )
            }
            PimdirQueryPredicate::Changed { since, until } => {
                let since = self.bind((*since).into());
                let until = self.bind((*until).into());
                format!(
                    "SELECT i.collection, i.seq {live} AND ({since} IS NULL OR i.changed >= {since}) \
                     AND ({until} IS NULL OR i.changed <= {until})"
                )
            }
            PimdirQueryPredicate::Id(PimdirQueryId::Seq(seq)) => {
                return Ok(format!("(seq = {})", self.bind((*seq).into())));
            }
            PimdirQueryPredicate::Id(PimdirQueryId::Link(link)) => {
                let link = self.bind(link.clone().into());
                format!("SELECT i.collection, i.seq {live} AND i.link_id = {link}")
            }
        };

        Ok(format!(
            "((akey, seq) IN (SELECT ifnull(c.account, ''), x.seq FROM ({placements}) x \
             JOIN store.collections c ON c.id = x.collection))"
        ))
    }

    /// The placements of `person:` (SEARCH §8): the contacts whose title
    /// matches, and every item naming one of their addresses in any role.
    fn person(&mut self, text: &PimdirQueryText) -> String {
        let at = self.ctes.len();
        let contacts = match fts(Some("title"), text) {
            None => "SELECT NULL, NULL WHERE 0".to_string(),
            Some(expression) => {
                let objects = self.embed(
                    search::MATCH_OBJECTS,
                    vec![(":match", expression.clone().into())],
                );
                let summaries =
                    self.embed(search::MATCH_SUMMARIES, vec![(":match", expression.into())]);
                format!(
                    "SELECT m.collection, m.seq FROM ({objects}) m \
                     JOIN store.collections c ON c.id = m.collection WHERE c.kind = 'text/vcard' \
                     UNION SELECT m.collection, m.seq FROM ({summaries}) m \
                     JOIN store.collections c ON c.id = m.collection WHERE c.kind = 'text/vcard'"
                )
            }
        };
        self.ctes.push(format!(
            "p{at}(collection, seq) AS MATERIALIZED ({contacts})"
        ));

        format!(
            "SELECT collection, seq FROM p{at} UNION \
             SELECT i.collection, i.seq FROM store.item_address a \
             JOIN store.items i ON i.collection = a.collection AND i.link_id = a.link_id \
             WHERE i.deleted = 0 AND i.retained_at IS NULL AND a.address IN ( \
               SELECT e.address FROM store.item_address e \
               JOIN store.items ei ON ei.collection = e.collection AND ei.link_id = e.link_id \
               WHERE e.role = 'email' AND (ei.collection, ei.seq) IN (SELECT collection, seq FROM p{at}))"
        )
    }

    /// Compiles a text term to a `t<n>` expression of its matches over
    /// bodies and, when the field exists there, over summaries, answering
    /// the condition over `live`.
    fn text(
        &mut self,
        column: Option<&str>,
        summary: bool,
        text: &PimdirQueryText,
        negated: bool,
    ) -> String {
        let at = self.ctes.len();
        let cte = match fts(column, text) {
            None => "SELECT NULL, NULL, NULL, NULL WHERE 0".to_string(),
            Some(expression) => {
                let objects = self.embed(
                    search::MATCH_OBJECTS,
                    vec![(":match", expression.clone().into())],
                );
                let mut union = format!(
                    "SELECT ifnull(c.account, ''), m.seq, 'body', m.rank FROM ({objects}) m \
                     JOIN store.collections c ON c.id = m.collection"
                );
                if summary {
                    let summaries =
                        self.embed(search::MATCH_SUMMARIES, vec![(":match", expression.into())]);
                    union += &format!(
                        " UNION ALL SELECT ifnull(c.account, ''), m.seq, 'summary', m.rank \
                         FROM ({summaries}) m JOIN store.collections c ON c.id = m.collection"
                    );
                }
                union
            }
        };
        // NOTE: materialised, so `bm25` runs in the full-text query that
        // ranks; flattened into the statement reading `rank`, a lone
        // `match_objects` would call it outside one, which FTS5 refuses.
        self.ctes.push(format!(
            "t{at}(akey, seq, src, rank) AS MATERIALIZED ({cte})"
        ));

        if !negated {
            self.positive.push(at);
            self.words.push(text.text.clone());
        }

        format!("((akey, seq) IN (SELECT akey, seq FROM t{at}))")
    }
}

/// The titles a hit's placements carry, joined for the `name` order.
const TITLES: &str = "FROM store.items i JOIN store.collections c ON c.id = i.collection
   LEFT JOIN store.mail_summary ms ON ms.collection = i.collection AND ms.link_id = i.link_id
   LEFT JOIN store.contact_summary cs ON cs.collection = i.collection AND cs.link_id = i.link_id
   LEFT JOIN store.event_summary es ON es.collection = i.collection AND es.link_id = i.link_id
   LEFT JOIN store.task_summary ts ON ts.collection = i.collection AND ts.link_id = i.link_id
   LEFT JOIN store.journal_summary js ON js.collection = i.collection AND js.link_id = i.link_id
   LEFT JOIN store.file_summary fs ON fs.collection = i.collection AND fs.link_id = i.link_id";

/// The collection media type of a kind.
pub(crate) fn media_type(kind: PimdirQueryKind) -> &'static str {
    match kind {
        PimdirQueryKind::Mail => "message/rfc822",
        PimdirQueryKind::Contact => "text/vcard",
        PimdirQueryKind::Event | PimdirQueryKind::Task | PimdirQueryKind::Journal => {
            "text/calendar"
        }
        PimdirQueryKind::File => file::KIND,
    }
}

/// The instants an absolute range spans, `[since, until)`, RFC 3339 at
/// midnight UTC, either side `None` open.
pub(crate) fn bounds(range: &PimdirQueryDateRange) -> (Option<String>, Option<String>) {
    let instant = |day: String| format!("{day}T00:00:00Z");
    (
        range
            .start
            .as_ref()
            .and_then(PimdirQueryDate::first_day)
            .map(instant),
        range
            .end
            .as_ref()
            .and_then(PimdirQueryDate::day_after)
            .map(instant),
    )
}

/// The FTS5 expression of a text term: the text as one escaped string, so
/// the tokenizer reads its words as a phrase and nothing as syntax, a
/// prefix starred, filtered to a column. `None` for a text holding no
/// word, which matches nothing.
fn fts(column: Option<&str>, text: &PimdirQueryText) -> Option<String> {
    if !text.text.chars().any(char::is_alphanumeric) {
        return None;
    }

    let mut expression = format!("\"{}\"", text.text.replace('"', "\"\""));
    if text.prefix {
        expression += " *";
    }
    Some(match column {
        Some(column) => format!("{{{column}}} : {expression}"),
        None => expression,
    })
}

/// Replaces a named parameter by a positional one wherever it stands
/// whole, so `:role` leaves `:roles` alone.
fn replace_param(text: &str, name: &str, position: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(name) {
        let after = &rest[at + name.len()..];
        let whole = !after
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        out.push_str(&rest[..at]);
        out.push_str(if whole { position } else { name });
        rest = after;
    }
    out.push_str(rest);
    out
}
