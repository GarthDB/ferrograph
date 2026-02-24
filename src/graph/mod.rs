//! Graph abstraction and storage.

use cozo::DataValue;

pub mod query;
pub mod schema;
pub mod store;

pub use query::Query;
pub use schema::{EdgeType, NodeId, NodeType};
pub use store::Store;

/// Cozo string values are often quoted in output; strip surrounding quotes.
pub(crate) fn cozo_str(v: &DataValue) -> String {
    v.to_string().trim_matches('"').to_string()
}
