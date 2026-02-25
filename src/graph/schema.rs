//! Node and edge type definitions for the code graph.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Unique identifier for a graph node (e.g. `path#line:col` or `path::name` placeholder).
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct NodeId(pub(crate) String);

impl NodeId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

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

impl FromStr for NodeType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "file" => Self::File,
            "module" => Self::Module,
            "function" => Self::Function,
            "struct" => Self::Struct,
            "enum" => Self::Enum,
            "trait" => Self::Trait,
            "impl" => Self::Impl,
            "type_alias" => Self::TypeAlias,
            "const" => Self::Const,
            "static" => Self::Static,
            "macro" => Self::Macro,
            "crate_root" => Self::CrateRoot,
            _ => return Err(()),
        })
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

impl FromStr for EdgeType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "contains" => Self::Contains,
            "imports" => Self::Imports,
            "calls" => Self::Calls,
            "references" => Self::References,
            "implements_trait" => Self::ImplementsTrait,
            "owns" => Self::Owns,
            "borrows" => Self::Borrows,
            "expands_to" => Self::ExpandsTo,
            "uses_unsafe" => Self::UsesUnsafe,
            "lifetime_scope" => Self::LifetimeScope,
            "changes_with" => Self::ChangesWith,
            _ => return Err(()),
        })
    }
}
