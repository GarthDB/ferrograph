//! Node and edge type definitions for the code graph.

use serde::{Deserialize, Serialize};

/// Unique identifier for a graph node.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NodeId(pub String);

/// Type of a graph node.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    File,
    Module,
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    TypeAlias,
    Const,
    Static,
    Macro,
    CrateRoot,
}

/// Type of a directed edge between nodes.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Contains,
    Imports,
    Calls,
    References,
    ImplementsTrait,
    Owns,
    Borrows,
    ExpandsTo,
    UsesUnsafe,
    LifetimeScope,
    /// Files that changed together in git history (Phase 10).
    ChangesWith,
}
