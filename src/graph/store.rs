//! CozoDB-backed graph storage.

use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::Result;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use crate::graph::query::Query;
use crate::graph::schema::{EdgeType, NodeId, NodeType};
use crate::graph::unquote_datavalue;

fn cozo_err(e: &cozo::Error) -> anyhow::Error {
    anyhow::anyhow!("{e:#}")
}

/// In-memory graph store using `CozoDB`.
pub struct Store {
    db: DbInstance,
}

impl Store {
    /// Create a new in-memory store.
    ///
    /// # Errors
    /// Fails if the Cozo in-memory engine cannot be initialized.
    pub fn new_memory() -> Result<Self> {
        let db = DbInstance::new("mem", "", Default::default()).map_err(|e| cozo_err(&e))?;
        let store = Store { db };
        store.init_schema()?;
        Ok(store)
    }

    /// Create or open a persistent store at the given path.
    ///
    /// # Errors
    /// Fails if the `SQLite` backend cannot be opened or the schema cannot be created.
    pub fn new_persistent(path: &std::path::Path) -> Result<Self> {
        let path_str = path.to_string_lossy();
        let db = DbInstance::new("sqlite", path_str.as_ref(), Default::default())
            .map_err(|e| cozo_err(&e))?;
        let store = Store { db };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        // Nodes: idempotent create (ignore error if relation already exists on reopen)
        self.db
            .run_script(
                "%ignore_error { :create nodes { id: String => type: String, payload: String? } }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        // Edges: composite key (from_id, to_id, edge_type)
        self.db
            .run_script(
                "%ignore_error { :create edges { from_id: String, to_id: String, edge_type: String } }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        // Dead functions (populated by pipeline phase 9)
        self.db
            .run_script(
                "%ignore_error { :create dead_functions { id: String } }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Insert a node.
    ///
    /// # Errors
    /// Fails if the Cozo script or serialization fails.
    pub fn put_node(&self, id: &NodeId, node_type: &NodeType, payload: Option<&str>) -> Result<()> {
        let type_str = node_type.to_string();
        let payload_val = payload.map_or(DataValue::Null, DataValue::from);
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id.as_str()));
        params.insert("type".to_string(), DataValue::from(type_str.as_str()));
        params.insert("payload".to_string(), payload_val);
        self.db
            .run_script(
                "?[id, type, payload] <- [[$id, $type, $payload]] :put nodes { id => type, payload }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Insert an edge.
    ///
    /// # Errors
    /// Fails if the Cozo script or serialization fails.
    pub fn put_edge(&self, from: &NodeId, to: &NodeId, edge_type: &EdgeType) -> Result<()> {
        let type_str = edge_type.to_string();
        let mut params = BTreeMap::new();
        params.insert("from_id".to_string(), DataValue::from(from.as_str()));
        params.insert("to_id".to_string(), DataValue::from(to.as_str()));
        params.insert("edge_type".to_string(), DataValue::from(type_str.as_str()));
        self.db
            .run_script(
                "?[from_id, to_id, edge_type] <- [[$from_id, $to_id, $edge_type]] :put edges { from_id, to_id, edge_type }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Insert multiple nodes in one script (more efficient than repeated `put_node`).
    ///
    /// # Errors
    /// Fails if the Cozo script or serialization fails.
    pub fn put_nodes_batch(&self, nodes: &[(NodeId, NodeType, Option<&str>)]) -> Result<()> {
        const CHUNK: usize = 100;
        for chunk in nodes.chunks(CHUNK) {
            let mut rows = Vec::with_capacity(chunk.len());
            let mut params = BTreeMap::new();
            for (i, (id, node_type, payload)) in chunk.iter().enumerate() {
                let type_str = node_type.to_string();
                let payload_val = payload.map_or(DataValue::Null, DataValue::from);
                params.insert(format!("id{i}"), DataValue::from(id.as_str()));
                params.insert(format!("type{i}"), DataValue::from(type_str.as_str()));
                params.insert(format!("payload{i}"), payload_val);
                rows.push(format!("[$id{i}, $type{i}, $payload{i}]"));
            }
            let script = format!(
                "?[id, type, payload] <- [{}] :put nodes {{ id => type, payload }}",
                rows.join(", ")
            );
            self.db
                .run_script(&script, params, ScriptMutability::Mutable)
                .map_err(|e| cozo_err(&e))?;
        }
        Ok(())
    }

    /// Insert multiple edges in one script (more efficient than repeated `put_edge`).
    ///
    /// # Errors
    /// Fails if the Cozo script or serialization fails.
    pub fn put_edges_batch(&self, edges: &[(NodeId, NodeId, EdgeType)]) -> Result<()> {
        const CHUNK: usize = 100;
        for chunk in edges.chunks(CHUNK) {
            let mut rows = Vec::with_capacity(chunk.len());
            let mut params = BTreeMap::new();
            for (i, (from, to, edge_type)) in chunk.iter().enumerate() {
                let type_str = edge_type.to_string();
                params.insert(format!("from_id{i}"), DataValue::from(from.as_str()));
                params.insert(format!("to_id{i}"), DataValue::from(to.as_str()));
                params.insert(format!("edge_type{i}"), DataValue::from(type_str.as_str()));
                rows.push(format!("[$from_id{i}, $to_id{i}, $edge_type{i}]"));
            }
            let script = format!(
                "?[from_id, to_id, edge_type] <- [{}] :put edges {{ from_id, to_id, edge_type }}",
                rows.join(", ")
            );
            self.db
                .run_script(&script, params, ScriptMutability::Mutable)
                .map_err(|e| cozo_err(&e))?;
        }
        Ok(())
    }

    /// Return the number of nodes (without loading all rows).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn node_count(&self) -> Result<usize> {
        let result = self
            .db
            .run_script(
                "?[count(id)] := *nodes[id, type, payload]",
                BTreeMap::new(),
                ScriptMutability::Immutable,
            )
            .map_err(|e| cozo_err(&e))?;
        let n: i64 = result
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(DataValue::get_int)
            .unwrap_or(0);
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Return the number of edges (without loading all rows).
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn edge_count(&self) -> Result<usize> {
        let result = self
            .db
            .run_script(
                "?[count(from_id)] := *edges[from_id, to_id, edge_type]",
                BTreeMap::new(),
                ScriptMutability::Immutable,
            )
            .map_err(|e| cozo_err(&e))?;
        let n: i64 = result
            .rows
            .first()
            .and_then(|r| r.first())
            .and_then(DataValue::get_int)
            .unwrap_or(0);
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Remove a single edge by key.
    ///
    /// # Errors
    /// Fails if the Cozo script or serialization fails.
    pub fn remove_edge(&self, from: &NodeId, to: &NodeId, edge_type: &EdgeType) -> Result<()> {
        let type_str = edge_type.to_string();
        let mut params = BTreeMap::new();
        params.insert("from_id".to_string(), DataValue::from(from.as_str()));
        params.insert("to_id".to_string(), DataValue::from(to.as_str()));
        params.insert("edge_type".to_string(), DataValue::from(type_str.as_str()));
        self.db
            .run_script(
                "?[from_id, to_id, edge_type] <- [[$from_id, $to_id, $edge_type]] :rm edges { from_id, to_id, edge_type }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Run a Datalog script (read-only).
    ///
    /// # Errors
    /// Fails if the script is invalid or execution fails.
    pub fn run_query(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
    ) -> Result<NamedRows> {
        let result = self
            .db
            .run_script(script, params, ScriptMutability::Immutable)
            .map_err(|e| cozo_err(&e))?;
        Ok(result)
    }

    /// Remove all nodes, edges, and `dead_functions` in one script (atomic for Cozo).
    ///
    /// # Errors
    /// Fails if the Cozo script fails.
    pub fn clear(&self) -> Result<()> {
        self.db
            .run_script(
                r"
                { ?[id] := *nodes[id, type, payload]; :rm nodes { id } }
                { ?[from_id, to_id, edge_type] := *edges[from_id, to_id, edge_type]; :rm edges { from_id, to_id, edge_type } }
                { ?[id] := *dead_functions[id]; :rm dead_functions { id } }
                ",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Clear the `dead_functions` relation (called by `clear()` and before repopulating).
    ///
    /// # Errors
    /// Fails if the Cozo script fails.
    pub fn clear_dead_functions(&self) -> Result<()> {
        self.db
            .run_script(
                "{ ?[id] := *dead_functions[id]; :rm dead_functions { id } }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Insert a single id into the `dead_functions` relation.
    ///
    /// # Errors
    /// Fails if the Cozo script fails.
    pub fn put_dead_function(&self, id: &str) -> Result<()> {
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));
        self.db
            .run_script(
                "?[id] <- [[$id]] :put dead_functions { id }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        Ok(())
    }

    /// Copy all nodes, edges, and `dead_functions` from another store into this one.
    /// Caller should typically call `clear()` first. Used by watch to replace
    /// contents after a successful pipeline run into a temp store.
    ///
    /// # Errors
    /// Fails if queries or writes fail.
    pub fn copy_from(&self, other: &Store) -> Result<()> {
        let nodes = Query::all_nodes(other)?;
        let batch: Vec<(NodeId, NodeType, Option<String>)> = nodes
            .rows
            .iter()
            .filter_map(|row| {
                let id = row.first().map(unquote_datavalue)?;
                let type_str = row.get(1).map(unquote_datavalue)?;
                let payload = row.get(2).and_then(|v| {
                    if matches!(v, DataValue::Null) {
                        None
                    } else {
                        Some(unquote_datavalue(v))
                    }
                });
                let node_type = NodeType::from_str(&type_str).ok()?;
                Some((NodeId(id), node_type, payload))
            })
            .collect();
        if !batch.is_empty() {
            let batch_refs: Vec<(NodeId, NodeType, Option<&str>)> = batch
                .iter()
                .map(|(id, ty, p)| (id.clone(), ty.clone(), p.as_deref()))
                .collect();
            self.put_nodes_batch(&batch_refs)?;
        }
        let edges = Query::all_edges(other)?;
        let edge_batch: Vec<_> = edges
            .rows
            .iter()
            .filter_map(|row| {
                let from_str = row.first().map(unquote_datavalue)?;
                let to_str = row.get(1).map(unquote_datavalue)?;
                let type_str = row.get(2).map(unquote_datavalue)?;
                let edge_type = EdgeType::from_str(&type_str).ok()?;
                Some((NodeId(from_str), NodeId(to_str), edge_type))
            })
            .collect();
        if !edge_batch.is_empty() {
            self.put_edges_batch(&edge_batch)?;
        }
        let dead = Query::stored_dead_functions(other)?;
        for id in &dead {
            self.put_dead_function(id)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::Store;
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};

    #[test]
    fn store_put_node_and_query() {
        let store = Store::new_memory().unwrap();
        let id = NodeId("n1".to_string());
        store
            .put_node(&id, &NodeType::Function, Some("foo"))
            .unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0].to_string().trim_matches('"'), "n1");
    }

    #[test]
    fn store_put_edge_and_query() {
        let store = Store::new_memory().unwrap();
        let from = NodeId("a".to_string());
        let to = NodeId("b".to_string());
        store.put_node(&from, &NodeType::Function, None).unwrap();
        store.put_node(&to, &NodeType::Function, None).unwrap();
        store.put_edge(&from, &to, &EdgeType::Calls).unwrap();
        let rows = Query::all_edges(&store).unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0][0].to_string().trim_matches('"'), "a");
        assert_eq!(rows.rows[0][1].to_string().trim_matches('"'), "b");
    }

    #[test]
    fn store_datalog_query() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("f".to_string()), &NodeType::Function, Some("main"))
            .unwrap();
        let script = "?[id, type, payload] := *nodes[id, type, payload]";
        let rows = store.run_query(script, BTreeMap::new()).unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert!(rows.rows[0][2].to_string().contains("main"));
    }

    #[test]
    fn store_clear_removes_all() {
        let store = Store::new_memory().unwrap();
        store
            .put_node(&NodeId("a".to_string()), &NodeType::Function, None)
            .unwrap();
        store
            .put_node(&NodeId("b".to_string()), &NodeType::Function, None)
            .unwrap();
        store.clear().unwrap();
        let rows = Query::all_nodes(&store).unwrap();
        assert!(rows.rows.is_empty());
    }

    #[test]
    fn store_persistent_reopen_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("reopen_test");
        {
            let store = Store::new_persistent(&db_path).unwrap();
            store
                .put_node(
                    &NodeId("persisted".to_string()),
                    &NodeType::Function,
                    Some("survives"),
                )
                .unwrap();
        }
        let store2 = Store::new_persistent(&db_path).unwrap();
        let rows = Query::all_nodes(&store2).unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert!(rows.rows[0][0].to_string().contains("persisted"));
        assert!(rows.rows[0][2].to_string().contains("survives"));
    }
}
