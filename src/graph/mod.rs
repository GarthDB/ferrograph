//! Graph abstraction and storage.

use cozo::DataValue;
use cozo::Num;
use serde_json::Number;
use serde_json::Value as JsonValue;

pub mod query;
pub mod schema;
pub mod store;

pub use query::{EdgeEndpoint, ModuleEdge, NodeInfo, Query};
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
        DataValue::Json(j) => j.0.clone(),
        DataValue::Validity(_) | DataValue::Set(_) | DataValue::Regex(_) | DataValue::Bot => {
            JsonValue::String(v.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::datavalue_to_json;
    use cozo::DataValue;

    #[test]
    fn datavalue_to_json_null() {
        let j = datavalue_to_json(&DataValue::Null);
        assert!(j.is_null());
    }

    #[test]
    fn datavalue_to_json_bool() {
        let j = datavalue_to_json(&DataValue::Bool(true));
        assert_eq!(j.as_bool(), Some(true));
    }

    #[test]
    fn datavalue_to_json_int() {
        let j = datavalue_to_json(&DataValue::from(42_i64));
        assert_eq!(j.as_i64(), Some(42));
    }

    #[test]
    fn datavalue_to_json_float_nan_becomes_null() {
        let j = datavalue_to_json(&DataValue::Num(cozo::Num::Float(f64::NAN)));
        assert!(j.is_null());
    }

    #[test]
    fn datavalue_to_json_float_infinity_becomes_null() {
        let j = datavalue_to_json(&DataValue::Num(cozo::Num::Float(f64::INFINITY)));
        assert!(j.is_null());
    }

    #[test]
    fn datavalue_to_json_str() {
        let j = datavalue_to_json(&DataValue::from("hello"));
        assert_eq!(j.as_str(), Some("hello"));
    }

    #[test]
    fn datavalue_to_json_list() {
        let list = DataValue::List(vec![DataValue::from(1), DataValue::from("x")]);
        let j = datavalue_to_json(&list);
        let arr = j.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[1].as_str(), Some("x"));
    }

    #[test]
    fn datavalue_to_json_bytes() {
        let b = DataValue::Bytes(vec![1_u8, 2, 3]);
        let j = datavalue_to_json(&b);
        let arr = j.as_array().expect("array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[1].as_i64(), Some(2));
        assert_eq!(arr[2].as_i64(), Some(3));
    }

    #[test]
    fn datavalue_to_json_bot_fallback_stringified() {
        let j = datavalue_to_json(&DataValue::Bot);
        assert!(j.is_string());
        assert!(!j.as_str().unwrap_or("").is_empty());
    }
}
