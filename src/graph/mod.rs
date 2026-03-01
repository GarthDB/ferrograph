//! Graph abstraction and storage.

use cozo::DataValue;

pub mod query;
pub mod schema;
pub mod store;

pub use query::{EdgeEndpoint, NodeInfo, Query};
pub use schema::{EdgeType, NodeId, NodeType};
pub use store::Store;

/// Extract string from a Cozo `DataValue` without mangling embedded quotes.
/// For `Str` variant returns the inner string; for other variants falls back to Display.
pub(crate) fn unquote_datavalue(v: &DataValue) -> String {
    match v {
        DataValue::Str(s) => s.to_string(),
        _ => v.to_string(),
    }
}
