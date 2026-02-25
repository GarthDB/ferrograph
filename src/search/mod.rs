//! Text search (substring match) over node payloads.
//!
//! Uses Cozo's `str_includes` over node payloads. Optional `search` feature is reserved
//! for future BM25/vector (e.g. fastembed-rs) when available.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;

use crate::graph::{unquote_datavalue, Store};

fn payload_from_row(row: &[DataValue], idx: usize) -> Option<String> {
    row.get(idx).and_then(|v| {
        if matches!(v, DataValue::Null) {
            None
        } else {
            Some(unquote_datavalue(v))
        }
    })
}

/// Run a text search over node payloads (substring match).
///
/// If `case_insensitive` is true, matching is done after lowercasing both query and payload.
///
/// # Errors
/// Fails if the store query fails.
pub fn text_search(
    store: &Store,
    query: &str,
    case_insensitive: bool,
) -> Result<Vec<(String, String, Option<String>)>> {
    let mut params = BTreeMap::new();
    let script = if case_insensitive {
        params.insert("q".to_string(), DataValue::from(query.to_lowercase()));
        r"
        ?[id, type, payload] := *nodes[id, type, payload],
          payload != null,
          str_includes(lowercase(payload), $q)
        "
    } else {
        params.insert("q".to_string(), DataValue::from(query));
        r"
        ?[id, type, payload] := *nodes[id, type, payload],
          payload != null,
          str_includes(payload, $q)
        "
    };
    let result = store.run_query(script.trim(), params)?;
    let rows = result
        .rows
        .iter()
        .map(|row| {
            let id = row.first().map(unquote_datavalue).unwrap_or_default();
            let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
            let payload = payload_from_row(row, 2);
            (id, type_val, payload)
        })
        .collect();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use crate::graph::schema::{NodeId, NodeType};
    use crate::graph::Store;

    use super::text_search;

    #[test]
    fn text_search_substring_match() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId("p#1:1".to_string()),
                &NodeType::Function,
                Some("hello_world"),
            )
            .unwrap();
        store
            .put_node(
                &NodeId("p#2:1".to_string()),
                &NodeType::Function,
                Some("other"),
            )
            .unwrap();
        let rows = text_search(&store, "hello", false).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2.as_deref(), Some("hello_world"));
    }

    #[test]
    fn text_search_case_insensitive() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId("p#1:1".to_string()),
                &NodeType::Function,
                Some("Main"),
            )
            .unwrap();
        let rows = text_search(&store, "main", true).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2.as_deref(), Some("Main"));
    }
}
