//! Node and edge type definitions for the code graph.

use std::fmt;

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

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File => write!(f, "file"),
            Self::Module => write!(f, "module"),
            Self::Function => write!(f, "function"),
            Self::Struct => write!(f, "struct"),
            Self::Enum => write!(f, "enum"),
            Self::Trait => write!(f, "trait"),
            Self::Impl => write!(f, "impl"),
            Self::TypeAlias => write!(f, "type_alias"),
            Self::Const => write!(f, "const"),
            Self::Static => write!(f, "static"),
            Self::Macro => write!(f, "macro"),
            Self::CrateRoot => write!(f, "crate_root"),
        }
    }
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

impl fmt::Display for EdgeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contains => write!(f, "contains"),
            Self::Imports => write!(f, "imports"),
            Self::Calls => write!(f, "calls"),
            Self::References => write!(f, "references"),
            Self::ImplementsTrait => write!(f, "implements_trait"),
            Self::Owns => write!(f, "owns"),
            Self::Borrows => write!(f, "borrows"),
            Self::ExpandsTo => write!(f, "expands_to"),
            Self::UsesUnsafe => write!(f, "uses_unsafe"),
            Self::LifetimeScope => write!(f, "lifetime_scope"),
            Self::ChangesWith => write!(f, "changes_with"),
        }
    }
}
