//! Common graph queries.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;
use cozo::NamedRows;
use serde::Serialize;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::{unquote_datavalue, Store};

/// Endpoint of an edge (the other node) for `node_info`.
#[derive(Debug, Clone, Serialize)]
pub struct EdgeEndpoint {
    pub id: String,
    pub edge_type: String,
    pub node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
}

/// Full node details plus edges for `node_info`.
#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    pub id: String,
    pub node_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    pub outgoing_edges: Vec<EdgeEndpoint>,
    pub incoming_edges: Vec<EdgeEndpoint>,
}

/// Extract optional string from a Cozo payload cell (Null → None).
fn optional_payload(v: &DataValue) -> Option<String> {
    if matches!(v, DataValue::Null) {
        None
    } else {
        Some(unquote_datavalue(v))
    }
}

/// Extract (id, type, payload) from a row of at least 3 columns (node table shape).
fn extract_node_triple(row: &[DataValue]) -> (String, String, Option<String>) {
    let id = row.first().map(unquote_datavalue).unwrap_or_default();
    let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
    let payload = row.get(2).and_then(optional_payload);
    (id, type_val, payload)
}

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

    /// Return nodes in the "blast radius" of the given node: immediate neighbors (both
    /// directions) plus all nodes reachable by following edges forward only (Calls, Contains,
    /// References, `ChangesWith`). Recursive expansion is forward-only to avoid inflating to the
    /// full connected component. Returns (id, type, payload) for each reachable node.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn blast_radius(
        store: &Store,
        from_id: &str,
    ) -> Result<Vec<(String, String, Option<String>)>> {
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
            ?[id, type, payload] := reachable[id], *nodes[id, type, payload], id != $from
            :limit {BLAST_RADIUS_LIMIT}
            "#
        );
        let rows = store.run_query(script.trim(), params)?;
        let results: Vec<(String, String, Option<String>)> = rows
            .rows
            .iter()
            .map(|row| extract_node_triple(row))
            .collect();
        Ok(results)
    }

    /// Return nodes that call the given node (reverse call graph). Optionally limited by depth.
    /// depth 1 = direct callers only; depth > 1 = transitive callers up to N hops.
    /// Returns (id, type, payload) for each caller.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn callers(
        store: &Store,
        target_id: &str,
        depth: u32,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        let edge_calls = EdgeType::Calls.to_string();
        let mut frontier: Vec<String> = vec![target_id.to_string()];
        let mut seen = std::collections::HashSet::new();
        seen.insert(target_id.to_string());
        let mut all_callers: Vec<(String, String, Option<String>)> = Vec::new();
        let max_rounds = depth.max(1) as usize;

        for _ in 0..max_rounds {
            if frontier.is_empty() {
                break;
            }
            let frontier_values: Vec<DataValue> = frontier
                .iter()
                .map(|s| DataValue::from(s.as_str()))
                .collect();
            let mut params = BTreeMap::new();
            params.insert("frontier".to_string(), DataValue::List(frontier_values));
            let script = format!(
                r#"
                ?[caller_id, type, payload] := *edges[caller_id, callee_id, "{edge_calls}"],
                  callee_id in $frontier,
                  *nodes[caller_id, type, payload]
                "#
            );
            let rows = store.run_query(script.trim(), params)?;
            frontier = Vec::new();
            for row in &rows.rows {
                let (id, type_val, payload) = extract_node_triple(row);
                if seen.insert(id.clone()) {
                    frontier.push(id.clone());
                    all_callers.push((id, type_val, payload));
                }
            }
        }
        Ok(all_callers)
    }

    /// Return full info for a node: type, payload, and all outgoing/incoming edges with endpoint details.
    /// Returns `None` if the node does not exist.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn node_info(store: &Store, node_id: &str) -> Result<Option<NodeInfo>> {
        let mut params = BTreeMap::new();
        params.insert("node_id".to_string(), DataValue::from(node_id));
        let node_rows = store.run_query(
            "?[id, type, payload] := *nodes[id, type, payload], id = $node_id",
            params.clone(),
        )?;
        let (id, node_type, payload) = match node_rows.rows.first() {
            Some(row) => extract_node_triple(row),
            None => return Ok(None),
        };
        let outgoing = {
            let rows = store.run_query(
                r"
                ?[to_id, edge_type, to_type, to_payload] := *edges[$node_id, to_id, edge_type],
                  *nodes[to_id, to_type, to_payload]
                ",
                params.clone(),
            )?;
            rows.rows
                .iter()
                .map(|row| EdgeEndpoint {
                    id: row.first().map(unquote_datavalue).unwrap_or_default(),
                    edge_type: row.get(1).map(unquote_datavalue).unwrap_or_default(),
                    node_type: row.get(2).map(unquote_datavalue).unwrap_or_default(),
                    payload: row.get(3).and_then(optional_payload),
                })
                .collect()
        };
        let incoming = {
            let rows = store.run_query(
                r"
                ?[from_id, edge_type, from_type, from_payload] := *edges[from_id, $node_id, edge_type],
                  *nodes[from_id, from_type, from_payload]
                ",
                params,
            )?;
            rows.rows
                .iter()
                .map(|row| EdgeEndpoint {
                    id: row.first().map(unquote_datavalue).unwrap_or_default(),
                    edge_type: row.get(1).map(unquote_datavalue).unwrap_or_default(),
                    node_type: row.get(2).map(unquote_datavalue).unwrap_or_default(),
                    payload: row.get(3).and_then(optional_payload),
                })
                .collect()
        };
        Ok(Some(NodeInfo {
            id,
            node_type,
            payload,
            outgoing_edges: outgoing,
            incoming_edges: incoming,
        }))
    }

    /// Return impl nodes that implement the given trait (by name substring match).
    /// Uses `ImplementsTrait` edges from impl to trait.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn trait_implementors(
        store: &Store,
        trait_name: &str,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        let mut params = BTreeMap::new();
        params.insert("trait_name".to_string(), DataValue::from(trait_name));
        let script = r#"
            ?[impl_id, impl_type, impl_payload] := *nodes[trait_id, type, payload],
              type = "trait",
              str_includes(payload, $trait_name),
              *edges[impl_id, trait_id, "implements_trait"],
              *nodes[impl_id, impl_type, impl_payload]
            :limit 500
        "#;
        let rows = store.run_query(script.trim(), params)?;
        let results: Vec<(String, String, Option<String>)> = rows
            .rows
            .iter()
            .map(|row| extract_node_triple(row))
            .collect();
        Ok(results)
    }

    /// Return the module containment graph: edges (`from_id`, `to_id`, `from_type`, `to_type`) for
    /// Contains relations between file, module, and `crate_root` nodes. Optional path prefix filter.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn module_graph(
        store: &Store,
        path_prefix: Option<&str>,
    ) -> Result<Vec<(String, String, String, String)>> {
        let script = r#"
            ?[from_id, to_id, from_type, to_type] := *edges[from_id, to_id, "contains"],
              *nodes[from_id, from_type, _],
              *nodes[to_id, to_type, _],
              from_type in ["file", "module", "crate_root"],
              to_type in ["file", "module"]
            :limit 10000
        "#;
        let rows = store.run_query(script.trim(), BTreeMap::new())?;
        let mut results: Vec<(String, String, String, String)> = rows
            .rows
            .iter()
            .map(|row| {
                (
                    row.first().map(unquote_datavalue).unwrap_or_default(),
                    row.get(1).map(unquote_datavalue).unwrap_or_default(),
                    row.get(2).map(unquote_datavalue).unwrap_or_default(),
                    row.get(3).map(unquote_datavalue).unwrap_or_default(),
                )
            })
            .collect();
        if let Some(prefix) = path_prefix {
            if !prefix.is_empty() {
                results.retain(|(from_id, to_id, _, _)| {
                    from_id.starts_with(prefix) || to_id.starts_with(prefix)
                });
            }
        }
        Ok(results)
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
        let ids_a: Vec<String> = from_a.iter().map(|(id, _, _)| id.clone()).collect();
        let ids_b: Vec<String> = from_b.iter().map(|(id, _, _)| id.clone()).collect();
        assert!(
            ids_a.contains(&"b".to_string()),
            "from a should reach b (seed)"
        );
        assert!(
            ids_b.contains(&"a".to_string()),
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
        let ids_a: Vec<String> = from_a.iter().map(|(id, _, _)| id.clone()).collect();
        let ids_c: Vec<String> = from_c.iter().map(|(id, _, _)| id.clone()).collect();
        assert!(
            ids_a.contains(&"b".to_string()) && ids_a.contains(&"c".to_string()),
            "from a should reach b and c (forward), got {ids_a:?}"
        );
        assert!(
            ids_c.contains(&"b".to_string()) && !ids_c.contains(&"a".to_string()),
            "from c should reach only b (seed), not transitive a, got {ids_c:?}"
        );
    }

    #[test]
    fn callers_direct_only_depth1() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId("callee".to_string()),
                &NodeType::Function,
                Some("callee"),
            )
            .unwrap();
        store
            .put_node(
                &NodeId("caller1".to_string()),
                &NodeType::Function,
                Some("caller1"),
            )
            .unwrap();
        store
            .put_node(
                &NodeId("caller2".to_string()),
                &NodeType::Function,
                Some("caller2"),
            )
            .unwrap();
        store
            .put_edge(
                &NodeId("caller1".to_string()),
                &NodeId("callee".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        store
            .put_edge(
                &NodeId("caller2".to_string()),
                &NodeId("callee".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let callers = Query::callers(&store, "callee", 1).unwrap();
        assert_eq!(callers.len(), 2, "two direct callers");
        let ids: Vec<String> = callers.iter().map(|(id, _, _)| id.clone()).collect();
        assert!(ids.contains(&"caller1".to_string()));
        assert!(ids.contains(&"caller2".to_string()));
    }

    #[test]
    fn node_info_returns_none_for_missing() {
        let store = Store::new_memory().unwrap();
        let info = Query::node_info(&store, "nonexistent").unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn node_info_returns_node_and_edges() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("n1".to_string()), &NodeType::Function, Some("foo"))
            .unwrap();
        store
            .put_node(&NodeId("n2".to_string()), &NodeType::Function, Some("bar"))
            .unwrap();
        store
            .put_edge(
                &NodeId("n1".to_string()),
                &NodeId("n2".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let info = Query::node_info(&store, "n1")
            .unwrap()
            .expect("node exists");
        assert_eq!(info.id, "n1");
        assert_eq!(info.node_type, "function");
        assert_eq!(info.payload.as_deref(), Some("foo"));
        assert_eq!(info.outgoing_edges.len(), 1);
        assert_eq!(info.outgoing_edges[0].id, "n2");
        assert_eq!(info.outgoing_edges[0].edge_type, "calls");
        assert_eq!(info.incoming_edges.len(), 0);
    }

    #[test]
    fn trait_implementors_empty_when_no_trait() {
        let store = Store::new_memory().unwrap();
        let impls = Query::trait_implementors(&store, "NoSuchTrait").unwrap();
        assert!(impls.is_empty());
    }

    #[test]
    fn module_graph_returns_contains_edges() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("file://a".to_string()), &NodeType::File, None)
            .unwrap();
        store
            .put_node(&NodeId("mod://b".to_string()), &NodeType::Module, None)
            .unwrap();
        store
            .put_edge(
                &NodeId("file://a".to_string()),
                &NodeId("mod://b".to_string()),
                &EdgeType::Contains,
            )
            .unwrap();
        let edges = Query::module_graph(&store, None).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "file://a");
        assert_eq!(edges[0].1, "mod://b");
    }
}
