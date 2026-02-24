//! Common graph queries.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;
use cozo::NamedRows;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::{cozo_str, Store};

/// Predefined and Datalog query execution.
pub struct Query;

impl Query {
    /// Return all nodes.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn all_nodes(store: &Store) -> Result<NamedRows> {
        store.run_query(
            "?[id, type, payload] := *nodes[id, type, payload]",
            BTreeMap::new(),
        )
    }

    /// Return all edges.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn all_edges(store: &Store) -> Result<NamedRows> {
        store.run_query(
            "?[from_id, to_id, edge_type] := *edges[from_id, to_id, edge_type]",
            BTreeMap::new(),
        )
    }

    /// Return dead function node ids from the stored relation (populated by the pipeline).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn dead_functions(store: &Store) -> Result<Vec<String>> {
        let rows = store.run_query("?[id] := *dead_functions[id]", BTreeMap::new())?;
        let ids: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(cozo_str)
            .collect();
        Ok(ids)
    }

    /// Return function node ids that are not reachable from any entry point.
    /// Entry points are functions with no incoming `calls` edge. Reachability is
    /// computed by following call edges forward (Datalog fixed-point).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn dead_function_ids(store: &Store) -> Result<Vec<String>> {
        let type_function = NodeType::Function.to_string();
        let edge_calls = EdgeType::Calls.to_string();
        let script = format!(
            r#"
            called[to] := *edges[_, to, "{edge_calls}"]
            entry[id] := *nodes[id, type, _], type = "{type_function}", not called[id]
            reachable[id] := entry[id]
            reachable[to] := reachable[from], *edges[from, to, "{edge_calls}"]
            ?[id] := *nodes[id, type, _], type = "{type_function}", not reachable[id]
            "#
        );
        let rows = store.run_query(script.trim(), BTreeMap::new())?;
        let ids: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(cozo_str)
            .collect();
        Ok(ids)
    }

    /// Return node ids reachable from the given node by following any outgoing edges.
    /// Used for "blast radius": what could break if this node changes (Datalog fixed-point).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn blast_radius(store: &Store, from_id: &str) -> Result<Vec<String>> {
        let from_trim = from_id.trim_matches('"');
        let mut params = BTreeMap::new();
        params.insert("from".to_string(), DataValue::from(from_trim));
        let script = r"
            reachable[to] := *edges[from, to, _], from = $from
            reachable[to] := reachable[from], *edges[from, to, _]
            ?[id] := reachable[id], id != $from
        ";
        let rows = store.run_query(script.trim(), params)?;
        let ids: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(cozo_str)
            .collect();
        Ok(ids)
    }
}
