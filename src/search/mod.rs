//! Hybrid search (BM25 + vector).
//!
//! Text search uses Cozo's `str_includes` over node payloads. Optional `search` feature
//! adds fastembed-rs and vector search (future).

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;

use crate::graph::{cozo_str, Store};

/// Run a text search over node payloads (substring match).
///
/// # Errors
/// Fails if the store query fails.
pub fn text_search(store: &Store, query: &str) -> Result<Vec<(String, String, Option<String>)>> {
    let mut params = BTreeMap::new();
    params.insert("q".to_string(), DataValue::from(query));
    let script = r"
        ?[id, type, payload] := *nodes[id, type, payload],
          payload != null,
          str_includes(payload, $q)
    ";
    let result = store.run_query(script.trim(), params)?;
    let rows = result
        .rows
        .iter()
        .map(|row| {
            let id = row.first().map(cozo_str).unwrap_or_default();
            let type_val = row.get(1).map(cozo_str).unwrap_or_default();
            let payload = row.get(2).map(cozo_str);
            let payload = payload.filter(|p| p != "null");
            (id, type_val, payload)
        })
        .collect();
    Ok(rows)
}
