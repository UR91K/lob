use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Opaque identifier for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

/// Opaque identifier for an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeId(pub u64);

/// A value stored in a node or edge attribute map.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
    Bytes(Vec<u8>),
    /// A reference to another node stored as an attribute value.
    NodeRef(NodeId),
}

/// Direction of edge traversal for graph queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Follow outgoing edges (from → to).
    Forward,
    /// Follow incoming edges (to → from).
    Reverse,
    /// Follow both directions simultaneously.
    Both,
}

/// The three edge types. Each has different lifetime semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// At most one per target. Target is deleted when this edge is dropped. No cycles permitted.
    Own,
    /// Many allowed. Shared borrow semantics. Keeps target alive. Prevents movement.
    Ref,
    /// Provenance / backlinks. No lifetime effect. Target may become a tombstone.
    Weak,
}

/// A directed edge between two nodes.
#[derive(Debug, Clone)]
pub struct Edge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub attrs: BTreeMap<String, Value>,
}

/// A node. Fields are split into kernel-enforced (immutable from userspace) and app-owned.
#[derive(Debug, Clone)]
pub struct Node {
    // ---- kernel-enforced ----
    pub id: NodeId,
    /// The single owner, or None if the node is unowned (persistent in the real system).
    pub owner: Option<NodeId>,
    /// Number of active Ref edges pointing TO this node.
    pub ref_count: u32,
    /// True when the node is dropped but Weak edges still reference it.
    /// Data and attrs are cleared; only the shell remains for tombstone checks.
    pub is_tombstone: bool,

    // ---- application-owned ----
    pub attrs: BTreeMap<String, Value>,
    pub data: Option<Vec<u8>>,
}

impl Node {
    pub(crate) fn new(id: NodeId, owner: Option<NodeId>) -> Self {
        Self {
            id,
            owner,
            ref_count: 0,
            is_tombstone: false,
            attrs: BTreeMap::new(),
            data: None,
        }
    }
}
