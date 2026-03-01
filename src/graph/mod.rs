//! Graph abstraction and storage.

use cozo::DataValue;
use cozo::Num;
use serde_json::Number;
use serde_json::Value as JsonValue;

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

/// Convert a Cozo `DataValue` to plain `serde_json::Value` (no `{"Str": "x"}` wrappers).
/// Used so MCP query tool returns JSON that is directly usable by clients.
pub fn datavalue_to_json(v: &DataValue) -> JsonValue {
    match v {
        DataValue::Null => JsonValue::Null,
        DataValue::Bool(b) => JsonValue::Bool(*b),
        DataValue::Num(n) => match n {
            Num::Int(i) => JsonValue::Number(Number::from(*i)),
            Num::Float(f) => Number::from_f64(*f).map_or(JsonValue::Null, JsonValue::Number),
        },
        DataValue::Str(s) => JsonValue::String(s.to_string()),
        DataValue::Bytes(b) => JsonValue::Array(
            b.iter()
                .map(|&x| JsonValue::Number(Number::from(x)))
                .collect(),
        ),
        DataValue::Uuid(u) => JsonValue::String(u.0.to_string()),
        DataValue::List(lst) => JsonValue::Array(lst.iter().map(datavalue_to_json).collect()),
        DataValue::Vec(vec) => {
            let arr: Vec<JsonValue> = match vec {
                cozo::Vector::F32(a) => a
                    .iter()
                    .map(|&x| {
                        Number::from_f64(f64::from(x)).map_or(JsonValue::Null, JsonValue::Number)
                    })
                    .collect(),
                cozo::Vector::F64(a) => a
                    .iter()
                    .map(|&x| Number::from_f64(x).map_or(JsonValue::Null, JsonValue::Number))
                    .collect(),
            };
            JsonValue::Array(arr)
        }
        DataValue::Json(j) => serde_json::to_value(&j.0).unwrap_or(JsonValue::Null),
        DataValue::Validity(_) | DataValue::Set(_) | DataValue::Regex(_) | DataValue::Bot => {
            JsonValue::String(v.to_string())
        }
    }
}
