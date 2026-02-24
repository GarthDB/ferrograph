//! Common graph queries.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::NamedRows;

use crate::graph::Store;

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

    /// Return function node ids that are not reachable from any entry point.
    /// Entry points are functions with no incoming `calls` edge. Reachability is
    /// computed by following call edges forward.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn dead_function_ids(store: &Store) -> Result<Vec<String>> {
        let edges = Self::all_edges(store)?;
        let mut call_to: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for row in &edges.rows {
            let edge_type = row
                .get(2)
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            if edge_type.contains("calls") {
                let from = row
                    .first()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();
                let to = row
                    .get(1)
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();
                let from_trim = from.trim_matches('"').to_string();
                let to_trim = to.trim_matches('"').to_string();
                call_to.entry(from_trim).or_default().push(to_trim);
            }
        }
        let nodes = Self::all_nodes(store)?;
        let mut function_ids: Vec<String> = Vec::new();
        let mut called: std::collections::HashSet<String> = std::collections::HashSet::new();
        for row in &edges.rows {
            let edge_type = row
                .get(2)
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            if edge_type.contains("calls") {
                let to = row
                    .get(1)
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();
                called.insert(to.trim_matches('"').to_string());
            }
        }
        for row in &nodes.rows {
            let type_val = row
                .get(1)
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            if type_val.contains("function") {
                let id = row
                    .first()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();
                function_ids.push(id.trim_matches('"').to_string());
            }
        }
        let entry_points: Vec<String> = function_ids
            .iter()
            .filter(|id| !called.contains(*id))
            .cloned()
            .collect();
        let mut reachable: std::collections::HashSet<String> =
            entry_points.iter().cloned().collect();
        let mut stack: Vec<String> = entry_points;
        while let Some(from) = stack.pop() {
            if let Some(callees) = call_to.get(&from) {
                for to in callees {
                    if reachable.insert(to.clone()) {
                        stack.push(to.clone());
                    }
                }
            }
        }
        let dead: Vec<String> = function_ids
            .into_iter()
            .filter(|id| !reachable.contains(id))
            .collect();
        Ok(dead)
    }

    /// Return node ids reachable from the given node by following any outgoing edges.
    /// Used for "blast radius": what could break if this node changes.
    ///
    /// # Errors
    /// Fails if the store query fails.
    pub fn blast_radius(store: &Store, from_id: &str) -> Result<Vec<String>> {
        let edges = Self::all_edges(store)?;
        let mut out_edges: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for row in &edges.rows {
            let from = row
                .first()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            let to = row
                .get(1)
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            let from_trim = from.trim_matches('"').to_string();
            let to_trim = to.trim_matches('"').to_string();
            out_edges.entry(from_trim).or_default().push(to_trim);
        }
        let from_trim = from_id.trim_matches('"');
        let mut reachable: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut stack = vec![from_trim.to_string()];
        while let Some(id) = stack.pop() {
            if reachable.insert(id.clone()) {
                if let Some(neighbors) = out_edges.get(&id) {
                    stack.extend(neighbors.iter().cloned());
                }
            }
        }
        reachable.remove(from_trim);
        Ok(reachable.into_iter().collect())
    }
}
