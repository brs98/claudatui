//! Search functionality for finding conversations.

pub mod engine;
pub mod types;

pub use engine::SearchEngine;
pub use types::{SearchFilterType, SearchQuery, SearchResult};
