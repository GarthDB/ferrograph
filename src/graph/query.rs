//! Common graph queries.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DataValue;
use cozo::NamedRows;

use crate::graph::schema::{EdgeType, NodeType};
use crate::graph::{unquote_datavalue, Store};

/// (id, type, payload, incoming\_edges, outgoing\_edges)
type NodeInfoResult = (
    String,
    String,
    Option<String>,
    Vec<(String, String)>,
    Vec<(String, String)>,
);

/// (parent\_id, parent\_type, parent\_payload, children\_ids)
type ModuleGraphEntry = (String, String, Option<String>, Vec<String>);

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

    /// Return (id, type) for the given node ids. Used for filtering dead code by node type.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn node_ids_to_types(store: &Store, ids: &[String]) -> Result<Vec<(String, String)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let id_list: Vec<DataValue> = ids.iter().map(|s| DataValue::from(s.as_str())).collect();
        let mut params = BTreeMap::new();
        params.insert("ids".to_string(), DataValue::List(id_list));
        let rows = store.run_query("?[id, type] := *nodes[id, type, _], id in $ids", params)?;
        let result: Vec<(String, String)> = rows
            .rows
            .iter()
            .map(|row| {
                (
                    row.first().map(unquote_datavalue).unwrap_or_default(),
                    row.get(1).map(unquote_datavalue).unwrap_or_default(),
                )
            })
            .collect();
        Ok(result)
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

    /// Return nodes in the "blast radius" of the given node (id, type, payload): immediate
    /// neighbors (both directions) plus all nodes reachable by following edges forward only
    /// (Calls, Contains, References, `ChangesWith`). Recursive expansion is forward-only to avoid
    /// inflating to the full connected component.
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
            ?[id, type, payload] := reachable[id], id != $from, *nodes[id, type, payload]
            :limit {BLAST_RADIUS_LIMIT}
            "#
        );
        let rows = store.run_query(script.trim(), params)?;
        let result: Vec<(String, String, Option<String>)> = rows
            .rows
            .iter()
            .map(|row| {
                let id = row.first().map(unquote_datavalue).unwrap_or_default();
                let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
                let payload = row.get(2).and_then(|v| {
                    if matches!(v, DataValue::Null) {
                        None
                    } else {
                        Some(unquote_datavalue(v))
                    }
                });
                (id, type_val, payload)
            })
            .collect();
        Ok(result)
    }

    /// Return nodes that call the given node (reverse call graph). Depth 1 = direct callers only;
    /// depth > 1 = transitive callers via fixed-point (Calls edges backward), limited to 500.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn callers(
        store: &Store,
        node_id: &str,
        depth: u32,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        const CALLERS_LIMIT: u64 = 500;
        let edge_calls = EdgeType::Calls.to_string();
        let mut params = BTreeMap::new();
        params.insert("target".to_string(), DataValue::from(node_id));

        let script = if depth <= 1 {
            format!(
                r#"
                ?[id, type, payload] := *edges[id, $target, "{edge_calls}"],
                  *nodes[id, type, payload]
                "#
            )
        } else {
            format!(
                r#"
                callers[id] := *edges[id, $target, "{edge_calls}"]
                callers[id] := *edges[id, mid, "{edge_calls}"], callers[mid]
                ?[id, type, payload] := callers[id], *nodes[id, type, payload]
                :limit {CALLERS_LIMIT}
                "#
            )
        };

        let rows = store.run_query(script.trim(), params)?;
        let result: Vec<(String, String, Option<String>)> = rows
            .rows
            .iter()
            .map(|row| {
                let id = row.first().map(unquote_datavalue).unwrap_or_default();
                let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
                let payload = row.get(2).and_then(|v| {
                    if matches!(v, DataValue::Null) {
                        None
                    } else {
                        Some(unquote_datavalue(v))
                    }
                });
                (id, type_val, payload)
            })
            .collect();
        Ok(result)
    }

    /// Return node metadata and immediate edges for a given node ID. If the node does not exist,
    /// returns None for node data and empty edge lists.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn node_info(store: &Store, node_id: &str) -> Result<Option<NodeInfoResult>> {
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(node_id));

        let node_rows = store.run_query(
            "?[id, type, payload] := *nodes[id, type, payload], id = $id",
            params.clone(),
        )?;
        let Some(node_row) = node_rows.rows.first() else {
            return Ok(None);
        };
        let nid = node_row.first().map(unquote_datavalue).unwrap_or_default();
        let ntype = node_row.get(1).map(unquote_datavalue).unwrap_or_default();
        let payload = node_row.get(2).and_then(|v| {
            if matches!(v, DataValue::Null) {
                None
            } else {
                Some(unquote_datavalue(v))
            }
        });

        let incoming = store.run_query(
            "?[from_id, edge_type] := *edges[from_id, to_id, edge_type], to_id = $id",
            params.clone(),
        )?;
        let incoming_edges: Vec<(String, String)> = incoming
            .rows
            .iter()
            .map(|r| {
                (
                    r.first().map(unquote_datavalue).unwrap_or_default(),
                    r.get(1).map(unquote_datavalue).unwrap_or_default(),
                )
            })
            .collect();

        let outgoing = store.run_query(
            "?[to_id, edge_type] := *edges[from_id, to_id, edge_type], from_id = $id",
            params,
        )?;
        let outgoing_edges: Vec<(String, String)> = outgoing
            .rows
            .iter()
            .map(|r| {
                (
                    r.first().map(unquote_datavalue).unwrap_or_default(),
                    r.get(1).map(unquote_datavalue).unwrap_or_default(),
                )
            })
            .collect();

        Ok(Some((nid, ntype, payload, incoming_edges, outgoing_edges)))
    }

    /// Return module containment tree: each node that has children, with (`node_id`, `node_type`,
    /// payload, children). If root is `Some`, only return the subtree under that node.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn module_graph(store: &Store, root: Option<&str>) -> Result<Vec<ModuleGraphEntry>> {
        let edge_contains = EdgeType::Contains.to_string();
        let script = if let Some(r) = root {
            let mut params = BTreeMap::new();
            params.insert("root".to_string(), DataValue::from(r));
            store.run_query(
                &format!(
                    r#"
                    descendant[id] := id = $root
                    descendant[child] := descendant[parent], *edges[parent, child, "{edge_contains}"]
                    ?[parent, parent_type, parent_payload, child] := descendant[parent],
                      *edges[parent, child, "{edge_contains}"],
                      *nodes[parent, parent_type, parent_payload]
                    "#
                ),
                params,
            )?
        } else {
            store.run_query(
                &format!(
                    r#"
                    ?[parent, parent_type, parent_payload, child] :=
                      *edges[parent, child, "{edge_contains}"],
                      *nodes[parent, parent_type, parent_payload]
                    "#
                ),
                BTreeMap::new(),
            )?
        };
        let mut by_parent: BTreeMap<String, (String, Option<String>, Vec<String>)> =
            BTreeMap::new();
        for row in &script.rows {
            let parent = row.first().map(unquote_datavalue).unwrap_or_default();
            let ptype = row.get(1).map(unquote_datavalue).unwrap_or_default();
            let ppayload = row.get(2).and_then(|v| {
                if matches!(v, DataValue::Null) {
                    None
                } else {
                    Some(unquote_datavalue(v))
                }
            });
            let child = row.get(3).map(unquote_datavalue).unwrap_or_default();
            let entry = by_parent
                .entry(parent.clone())
                .or_insert_with(|| (ptype.clone(), ppayload.clone(), Vec::new()));
            entry.2.push(child);
        }
        let result: Vec<ModuleGraphEntry> = by_parent
            .into_iter()
            .map(|(parent, (ptype, ppayload, children))| (parent, ptype, ppayload, children))
            .collect();
        Ok(result)
    }

    /// Return all impl blocks that implement the given trait (`ImplementsTrait` edges backward).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn trait_implementors(
        store: &Store,
        trait_node_id: &str,
    ) -> Result<Vec<(String, String, Option<String>)>> {
        let edge_impl = EdgeType::ImplementsTrait.to_string();
        let mut params = BTreeMap::new();
        params.insert("trait_id".to_string(), DataValue::from(trait_node_id));
        let rows = store.run_query(
            &format!(
                r#"
                ?[impl_id, type, payload] :=
                  *edges[impl_id, $trait_id, "{edge_impl}"],
                  *nodes[impl_id, type, payload]
                "#
            ),
            params,
        )?;
        let result: Vec<(String, String, Option<String>)> = rows
            .rows
            .iter()
            .map(|row| {
                let id = row.first().map(unquote_datavalue).unwrap_or_default();
                let type_val = row.get(1).map(unquote_datavalue).unwrap_or_default();
                let payload = row.get(2).and_then(|v| {
                    if matches!(v, DataValue::Null) {
                        None
                    } else {
                        Some(unquote_datavalue(v))
                    }
                });
                (id, type_val, payload)
            })
            .collect();
        Ok(result)
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
            from_a.iter().any(|(id, _, _)| id == "b"),
            "from a should reach b (seed)"
        );
        assert!(
            from_b.iter().any(|(id, _, _)| id == "a"),
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
        let ids_a: Vec<&str> = from_a.iter().map(|(id, _, _)| id.as_str()).collect();
        let ids_c: Vec<&str> = from_c.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(
            ids_a.contains(&"b") && ids_a.contains(&"c"),
            "from a should reach b and c (forward), got {ids_a:?}"
        );
        assert!(
            ids_c.contains(&"b") && !ids_c.contains(&"a"),
            "from c should reach only b (seed), not transitive a, got {ids_c:?}"
        );
    }

    #[test]
    fn callers_depth1_returns_direct_callers() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId("a".to_string()),
                &NodeType::Function,
                Some("caller_a"),
            )
            .unwrap();
        store
            .put_node(
                &NodeId("b".to_string()),
                &NodeType::Function,
                Some("target"),
            )
            .unwrap();
        store
            .put_edge(
                &NodeId("a".to_string()),
                &NodeId("b".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let callers = Query::callers(&store, "b", 1).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].0, "a");
        assert_eq!(callers[0].2.as_deref(), Some("caller_a"));
    }

    #[test]
    fn node_info_returns_node_and_edges() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("n1".to_string()), &NodeType::Function, Some("foo"))
            .unwrap();
        store
            .put_node(&NodeId("n2".to_string()), &NodeType::Function, None)
            .unwrap();
        store
            .put_edge(
                &NodeId("n2".to_string()),
                &NodeId("n1".to_string()),
                &EdgeType::Calls,
            )
            .unwrap();
        let info = Query::node_info(&store, "n1").unwrap();
        let (id, ntype, payload, inc, out) = info.expect("node should exist");
        assert_eq!(id, "n1");
        assert_eq!(ntype, "function");
        assert_eq!(payload.as_deref(), Some("foo"));
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].0, "n2");
        assert_eq!(inc[0].1, "calls");
        assert!(out.is_empty());
    }

    #[test]
    fn trait_implementors_returns_impls_for_trait() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(
                &NodeId("trait#1:1".to_string()),
                &NodeType::Trait,
                Some("MyTrait"),
            )
            .unwrap();
        store
            .put_node(
                &NodeId("impl#5:1".to_string()),
                &NodeType::Impl,
                Some("impl MyTrait for Foo"),
            )
            .unwrap();
        store
            .put_edge(
                &NodeId("impl#5:1".to_string()),
                &NodeId("trait#1:1".to_string()),
                &EdgeType::ImplementsTrait,
            )
            .unwrap();
        let impls = Query::trait_implementors(&store, "trait#1:1").unwrap();
        assert_eq!(impls.len(), 1);
        assert_eq!(impls[0].0, "impl#5:1");
        assert_eq!(impls[0].1, "impl");
        assert_eq!(impls[0].2.as_deref(), Some("impl MyTrait for Foo"));
    }
}
