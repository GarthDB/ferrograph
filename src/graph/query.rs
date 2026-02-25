//! Common graph queries.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;
use cozo::NamedRows;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::{unquote_datavalue, Store};

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
    pub fn stored_dead_functions(store: &Store) -> Result<Vec<String>> {
        let rows = store.run_query("?[id] := *dead_functions[id]", BTreeMap::new())?;
        let ids: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(unquote_datavalue)
            .collect();
        Ok(ids)
    }

    /// Return function node ids that are not reachable from known entry points.
    /// Entry points: functions named "main", and public functions (payload starting with "`pub::`").
    /// Reachability is computed by following Calls and References edges forward (Datalog fixed-point).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn compute_dead_functions(store: &Store) -> Result<Vec<String>> {
        let type_function = NodeType::Function.to_string();
        let edge_calls = EdgeType::Calls.to_string();
        let edge_refs = EdgeType::References.to_string();
        // Compute entry point ids in Rust (Cozo has no str_starts_with); then pass to Datalog.
        let all_fns = store.run_query(
            &format!("?[id, payload] := *nodes[id, type, payload], type = \"{type_function}\"",),
            BTreeMap::new(),
        )?;
        let entry_ids: Vec<DataValue> = all_fns
            .rows
            .iter()
            .filter_map(|row| {
                let id = row.first().map(unquote_datavalue)?;
                let payload = row.get(1).map(unquote_datavalue).unwrap_or_default();
                let is_entry = payload == "main"
                    || payload.starts_with("pub::")
                    || payload.starts_with("test::");
                is_entry.then(|| DataValue::from(id))
            })
            .collect();
        let mut params = BTreeMap::new();
        params.insert("entry_ids".to_string(), DataValue::List(entry_ids));
        let script = format!(
            r#"
            entry[id] := id in $entry_ids
            reachable[id] := entry[id]
            reachable[to] := reachable[from], *edges[from, to, "{edge_calls}"]
            reachable[to] := reachable[from], *edges[from, to, "{edge_refs}"]
            ?[id] := *nodes[id, type, _], type = "{type_function}", not reachable[id]
            "#
        );
        let rows = store.run_query(script.trim(), params)?;
        let ids: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(unquote_datavalue)
            .collect();
        Ok(ids)
    }

    /// Return node ids in the "blast radius" of the given node: immediate neighbors (both
    /// directions) plus all nodes reachable by following edges forward only (Calls, Contains,
    /// References, `ChangesWith`). Recursive expansion is forward-only to avoid inflating to the
    /// full connected component.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn blast_radius(store: &Store, from_id: &str) -> Result<Vec<String>> {
        const BLAST_RADIUS_LIMIT: u64 = 500;
        let edge_calls = EdgeType::Calls.to_string();
        let edge_contains = EdgeType::Contains.to_string();
        let edge_refs = EdgeType::References.to_string();
        let edge_changes = EdgeType::ChangesWith.to_string();
        let mut params = BTreeMap::new();
        params.insert("from".to_string(), DataValue::from(from_id));
        let script = format!(
            r#"
            seed[to] := *edges[from, to, et], from = $from, et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            seed[from] := *edges[from, to, et], to = $from, et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            reachable[id] := seed[id]
            reachable[to] := reachable[n], *edges[n, to, et], et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            ?[id] := reachable[id], id != $from
            :limit {BLAST_RADIUS_LIMIT}
            "#
        );
        let rows = store.run_query(script.trim(), params)?;
        let ids: Vec<String> = rows
            .rows
            .iter()
            .filter_map(|row| row.first())
            .map(unquote_datavalue)
            .collect();
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    use super::Query;

    #[test]
    fn dead_function_ids_marks_unreachable_from_main() {
        let store = Store::new_memory().unwrap();
        // main (entry) -> foo -> bar; baz is dead
        store
            .put_node(&NodeId::new("f#1:1"), &NodeType::Function, Some("main"))
            .unwrap();
        store
            .put_node(&NodeId::new("f#3:1"), &NodeType::Function, Some("foo"))
            .unwrap();
        store
            .put_node(&NodeId::new("f#5:1"), &NodeType::Function, Some("bar"))
            .unwrap();
        store
            .put_node(&NodeId::new("f#7:1"), &NodeType::Function, Some("baz"))
            .unwrap();
        store
            .put_edge(
                &NodeId::new("f#1:1"),
                &NodeId::new("f#3:1"),
                &EdgeType::Calls,
            )
            .unwrap();
        store
            .put_edge(
                &NodeId::new("f#3:1"),
                &NodeId::new("f#5:1"),
                &EdgeType::Calls,
            )
            .unwrap();
        let dead = Query::compute_dead_functions(&store).unwrap();
        assert!(
            dead.iter().any(|id| id == "f#7:1"),
            "baz (f#7:1) should be dead, got {dead:?}"
        );
    }

    #[test]
    fn test_function_is_entry_point_not_dead() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId::new("f#1:1"),
                &NodeType::Function,
                Some("test::my_test"),
            )
            .unwrap();
        let dead = Query::compute_dead_functions(&store).unwrap();
        assert!(
            !dead.iter().any(|id| id == "f#1:1"),
            "test::my_test (f#1:1) should not be dead (test is entry point), got {dead:?}"
        );
    }

    #[test]
    fn blast_radius_seed_includes_direct_neighbors() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("a".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_node(&NodeId("b".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_edge(
                &NodeId("a".to_string()),
                &NodeId("b".to_string()),
                &EdgeType::ChangesWith,
            )
            .unwrap();
        let from_a = Query::blast_radius(&store, "a").unwrap();
        let from_b = Query::blast_radius(&store, "b").unwrap();
        assert!(
            from_a.contains(&"b".to_string()),
            "from a should reach b (seed)"
        );
        assert!(
            from_b.contains(&"a".to_string()),
            "from b should reach a (seed, direct backward neighbor)"
        );
    }

    #[test]
    fn blast_radius_forward_only_recursion() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("a".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_node(&NodeId("b".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_node(&NodeId("c".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_edge(
                &NodeId("a".to_string()),
                &NodeId("b".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        store
            .put_edge(
                &NodeId("b".to_string()),
                &NodeId("c".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let from_a = Query::blast_radius(&store, "a").unwrap();
        let from_c = Query::blast_radius(&store, "c").unwrap();
        assert!(
            from_a.contains(&"b".to_string()) && from_a.contains(&"c".to_string()),
            "from a should reach b and c (forward), got {from_a:?}"
        );
        assert!(
            from_c.contains(&"b".to_string()) && !from_c.contains(&"a".to_string()),
            "from c should reach only b (seed), not transitive a, got {from_c:?}"
        );
    }
}
