//! CozoDB-backed graph storage.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use crate::graph::schema::{EdgeType, NodeId, NodeType};

fn cozo_err(e: &cozo::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
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
        // Nodes: id => type, payload
        self.db
            .run_script(
                ":create nodes { id: String => type: String, payload: String? }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        // Edges: composite key (from_id, to_id, edge_type)
        self.db
            .run_script(
                ":create edges { from_id: String, to_id: String, edge_type: String }",
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
        let type_str = serde_json::to_string(node_type)?;
        let payload_val = payload.map_or(DataValue::Null, DataValue::from);
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id.0.as_str()));
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
        let type_str = serde_json::to_string(edge_type)?;
        let mut params = BTreeMap::new();
        params.insert("from_id".to_string(), DataValue::from(from.0.as_str()));
        params.insert("to_id".to_string(), DataValue::from(to.0.as_str()));
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

    /// Remove all nodes and edges (for full re-index).
    ///
    /// # Errors
    /// Fails if the Cozo script fails.
    pub fn clear(&self) -> Result<()> {
        self.db
            .run_script(
                "{ ?[id] := *nodes[id, type, payload]; :rm nodes { id } }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
        self.db
            .run_script(
                "{ ?[from_id, to_id, edge_type] := *edges[from_id, to_id, edge_type]; :rm edges { from_id, to_id, edge_type } }",
                BTreeMap::new(),
                ScriptMutability::Mutable,
            )
            .map_err(|e| cozo_err(&e))?;
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
        assert!(rows.rows[0][0].to_string().contains("n1"));
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
        assert!(rows.rows[0][0].to_string().contains("a"));
        assert!(rows.rows[0][1].to_string().contains("b"));
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
}
