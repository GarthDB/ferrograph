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
    pub fn dead_functions(store: &Store) -> Result<Vec<String>> {
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
    /// Entry points are functions named "main" (and in future: pub at crate root, test fns).
    /// Reachability is computed by following call edges forward (Datalog fixed-point).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn dead_function_ids(store: &Store) -> Result<Vec<String>> {
        let type_function = NodeType::Function.to_string();
        let edge_calls = EdgeType::Calls.to_string();
        let script = format!(
            r#"
            entry[id] := *nodes[id, type, payload], type = "{type_function}", payload = "main"
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
            .map(unquote_datavalue)
            .collect();
        Ok(ids)
    }

    /// Return node ids reachable from the given node by following edges in both directions.
    /// Only follows Calls, Contains, References, and ChangesWith (not all edge types).
    /// Used for "blast radius": what could break if this node changes (Datalog fixed-point).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn blast_radius(store: &Store, from_id: &str) -> Result<Vec<String>> {
        let from_trim = from_id.trim_matches('"');
        let edge_calls = EdgeType::Calls.to_string();
        let edge_contains = EdgeType::Contains.to_string();
        let edge_refs = EdgeType::References.to_string();
        let edge_changes = EdgeType::ChangesWith.to_string();
        let mut params = BTreeMap::new();
        params.insert("from".to_string(), DataValue::from(from_trim));
        let script = format!(
            r#"
            reachable[to] := *edges[from, to, et], from = $from, et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            reachable[from] := *edges[from, to, et], to = $from, et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            reachable[to] := reachable[n], *edges[n, to, et], et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            reachable[from] := reachable[n], *edges[from, n, et], et in ["{edge_calls}", "{edge_contains}", "{edge_refs}", "{edge_changes}"]
            ?[id] := reachable[id], id != $from
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
        let dead = Query::dead_function_ids(&store).unwrap();
        assert!(
            dead.iter().any(|id| id == "f#7:1"),
            "baz (f#7:1) should be dead, got {dead:?}"
        );
    }

    #[test]
    fn blast_radius_bidirectional() {
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
        assert!(from_a.contains(&"b".to_string()), "from a should reach b");
        assert!(
            from_b.contains(&"a".to_string()),
            "from b should reach a (bidirectional)"
        );
    }
}
