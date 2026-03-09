//! Lifetime scope edges: items that declare explicit lifetime parameters get a self-loop
//! `LifetimeScope` edge (emitted in AST phase). No resolution phase—lifetimes are not
//! resolved to other node types. Elided lifetimes and outlives analysis would require rust-analyzer.

// No resolver function: lifetime_scope edges are created directly in ast.rs when we detect
// type_parameters containing lifetime_parameter on function_item, struct_item, enum_item,
// impl_item, trait_item.

#[cfg(test)]
mod tests {
    use crate::graph::query::Query;
    use crate::graph::schema::{EdgeType, NodeId, NodeType};
    use crate::graph::Store;

    #[test]
    fn lifetime_scope_edge_stored_and_queried() {
        let store = Store::new_memory().unwrap();
        let path = "src/lib.rs";
        let fn_id = NodeId::new(format!("{path}#5:1"));
        store
            .put_node(&fn_id, &NodeType::Function, Some("with_lt"))
            .unwrap();
        store
            .put_edge(&fn_id, &fn_id, &EdgeType::LifetimeScope)
            .unwrap();
        let edges = Query::all_edges(&store).unwrap();
        assert_eq!(edges.rows.len(), 1);
        let et = edges.rows[0][2].to_string().trim_matches('"').to_string();
        assert_eq!(et, "lifetime_scope");
    }
}
