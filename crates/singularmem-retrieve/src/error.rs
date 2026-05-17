//! Error type for the retrieve crate.

/// Alias for `std::result::Result<T, Error>` used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by `singularmem-retrieve` operations.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Underlying search-layer failure.
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),

    /// Underlying core-layer failure (e.g., `Store::get` on a deleted item).
    #[error("{0}")]
    Core(#[from] singularmem_core::Error),

    /// Query was empty or whitespace-only.
    #[error("query is empty; retrieval requires a non-empty query string")]
    EmptyQuery,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_error_message_explains_the_problem() {
        let e = Error::EmptyQuery;
        let msg = e.to_string();
        assert!(
            msg.contains("query is empty"),
            "message should explain the failure: got {msg:?}"
        );
        assert!(
            msg.contains("non-empty"),
            "message should tell user what to provide: got {msg:?}"
        );
    }
}
