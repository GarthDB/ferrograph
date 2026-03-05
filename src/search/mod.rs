//! Text search (substring match) over node payloads.
//!
//! Uses Cozo's `str_includes` over node payloads. Optional `search` feature is reserved
//! for future BM25/vector (e.g. fastembed-rs) when available.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;

use crate::graph::{unquote_datavalue, Store};

/// One row from a text search: (`node_id`, `node_type`, `optional_payload`).
pub type TextSearchRow = (String, String, Option<String>);

fn payload_from_row(row: &[DataValue], idx: usize) -> Option<String> {
    row.get(idx).and_then(|v| {
        if matches!(v, DataValue::Null) {
            None
        } else {
            Some(unquote_datavalue(v))
        }
    })
}

/// Run a text search over node payloads (substring match) with pagination.
///
/// If `case_insensitive` is true, matching is done after lowercasing both query and payload.
/// Limit and offset are applied in the query engine; returns the page of results and the total
/// match count.
///
/// # Errors
/// Fails if the store query fails.
pub fn text_search(
    store: &Store,
    query: &str,
    case_insensitive: bool,
    limit: usize,
    offset: usize,
) -> Result<(Vec<TextSearchRow>, usize)> {
    let mut params = BTreeMap::new();
    let (base_script, count_script) = if case_insensitive {
        params.insert("q".to_string(), DataValue::from(query.to_lowercase()));
        (
            r"
        ?[id, type, payload] := *nodes[id, type, payload],
          payload != null,
          str_includes(lowercase(payload), $q)
        ",
            r"
        ?[count(id)] := *nodes[id, type, payload],
          payload != null,
          str_includes(lowercase(payload), $q)
        ",
        )
    } else {
        params.insert("q".to_string(), DataValue::from(query));
        (
            r"
        ?[id, type, payload] := *nodes[id, type, payload],
          payload != null,
          str_includes(payload, $q)
        ",
            r"
        ?[count(id)] := *nodes[id, type, payload],
          payload != null,
          str_includes(payload, $q)
        ",
        )
    };

    let count_result = store.run_query(count_script.trim(), params.clone())?;
    let total: usize = count_result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(DataValue::get_int)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);

    let script = format!(
        "{}\n:limit {limit}\n:offset {offset}",
        base_script.trim(),
        limit = limit,
        offset = offset
    );
    let result = store.run_query(&script, params)?;
    let rows: Vec<_> = result
        .rows
        .iter()
        .map(|row| {
            let id = row.first().map(unquote_datavalue).unwrap_or_default();
            let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
            let payload = payload_from_row(row, 2);
            (id, type_val, payload)
        })
        .collect();
    Ok((rows, total))
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
        let (rows, total) = text_search(&store, "hello", false, 100, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
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
        let (rows, total) = text_search(&store, "main", true, 100, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(total, 1);
        assert_eq!(rows[0].2.as_deref(), Some("Main"));
    }

    #[test]
    fn text_search_pagination_returns_page_and_total() {
        let store = Store::new_memory().unwrap();
        for i in 1..=5 {
            store
                .put_node(
                    &NodeId(format!("p#{i}:1")),
                    &NodeType::Function,
                    Some("match"),
                )
                .unwrap();
        }
        let (page, total) = text_search(&store, "match", false, 2, 1).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(total, 5);
    }
}
