pub mod auth;

pub mod search;
pub use search::{
    parsing::{SearchResult, parse_results},
    search,
};
