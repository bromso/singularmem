//! `Retriever` composes `HybridSearcher` + `Store::get` into prompt-ready
//! memory blocks. The struct borrows references to both so callers retain
//! ownership of the underlying components.
