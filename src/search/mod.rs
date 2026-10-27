//! # Search
//!
//! The I/O-free half of pimdir SEARCH.md: what a search answers about
//! what it could not see ([`coverage`]), and, behind the `search`
//! feature, the query language parsed to an AST (`query`) and the
//! chunk boundaries extraction records (`chunk`). The index itself,
//! index.db and its refresh, is the client's
//! (`client::search`).
//!
//! [`coverage`] is outside the feature: the store's `LIKE` reads answer
//! the same coverage before any index exists.

#[cfg(feature = "search")]
pub mod chunk;
pub mod coverage;
#[cfg(feature = "search")]
pub mod query;
