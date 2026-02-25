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

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(Self::File),
            "module" => Ok(Self::Module),
            "function" => Ok(Self::Function),
            "struct" => Ok(Self::Struct),
            "enum" => Ok(Self::Enum),
            "trait" => Ok(Self::Trait),
            "impl" => Ok(Self::Impl),
            "type_alias" => Ok(Self::TypeAlias),
            "const" => Ok(Self::Const),
            "static" => Ok(Self::Static),
            "macro" => Ok(Self::Macro),
            "crate_root" => Ok(Self::CrateRoot),
            _ => Err(format!("unknown node type: {s:?}")),
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

impl FromStr for EdgeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contains" => Ok(Self::Contains),
            "imports" => Ok(Self::Imports),
            "calls" => Ok(Self::Calls),
            "references" => Ok(Self::References),
            "implements_trait" => Ok(Self::ImplementsTrait),
            "owns" => Ok(Self::Owns),
            "borrows" => Ok(Self::Borrows),
            "expands_to" => Ok(Self::ExpandsTo),
            "uses_unsafe" => Ok(Self::UsesUnsafe),
            "lifetime_scope" => Ok(Self::LifetimeScope),
            "changes_with" => Ok(Self::ChangesWith),
            _ => Err(format!("unknown edge type: {s:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{EdgeType, NodeType};

    #[test]
    fn node_type_display_from_str_roundtrip() {
        let variants = [
            NodeType::File,
            NodeType::Module,
            NodeType::Function,
            NodeType::Struct,
            NodeType::Enum,
            NodeType::Trait,
            NodeType::Impl,
            NodeType::TypeAlias,
            NodeType::Const,
            NodeType::Static,
            NodeType::Macro,
            NodeType::CrateRoot,
        ];
        for v in &variants {
            let s = v.to_string();
            let parsed = NodeType::from_str(&s).expect("parse node type");
            assert_eq!(format!("{v:?}"), format!("{parsed:?}"), "roundtrip {s}");
        }
    }

    #[test]
    fn edge_type_display_from_str_roundtrip() {
        let variants = [
            EdgeType::Contains,
            EdgeType::Imports,
            EdgeType::Calls,
            EdgeType::References,
            EdgeType::ImplementsTrait,
            EdgeType::Owns,
            EdgeType::Borrows,
            EdgeType::ExpandsTo,
            EdgeType::UsesUnsafe,
            EdgeType::LifetimeScope,
            EdgeType::ChangesWith,
        ];
        for v in &variants {
            let s = v.to_string();
            let parsed = EdgeType::from_str(&s).expect("parse edge type");
            assert_eq!(format!("{v:?}"), format!("{parsed:?}"), "roundtrip {s}");
        }
    }
}
