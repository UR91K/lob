use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::error::LobError;
use crate::node::{Edge, EdgeId, EdgeKind, Node, NodeId, Direction};

// ── Capability tokens ────────────────────────────────────────────────────────

/// Proof that you hold an exclusive write lease on a node.
/// Required by [`NodeStore::get_node_mut`]. Release with [`NodeStore::release_ref_mut`].
///
/// Note: in a future iteration these will be represented as nodes in the graph
/// so they are visible in the node browser (showing which process holds a lock).
/// For Phase 0, they are tracked in-memory for simplicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseToken {
    pub(crate) target: NodeId,
}

/// Proof that you hold a shared borrow (Ref edge) on a node.
/// Release with [`NodeStore::release_ref`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefToken {
    pub(crate) edge_id: EdgeId,
    pub(crate) target: NodeId,
}

/// A Weak edge handle. Use [`NodeStore::upgrade`] to attempt promotion to a [`RefToken`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeakToken {
    pub(crate) edge_id: EdgeId,
    pub(crate) target: NodeId,
}

// ── NodeStore ────────────────────────────────────────────────────────────────

/// The core LOB node store. Owns all nodes and edges and enforces all eight invariants
/// at every operation boundary.
pub struct NodeStore {
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeMap<EdgeId, Edge>,
    /// Outgoing edges indexed by source node.
    edges_from: BTreeMap<NodeId, Vec<EdgeId>>,
    /// Incoming edges indexed by target node.
    edges_to: BTreeMap<NodeId, Vec<EdgeId>>,
    /// Nodes that currently have an active ref_mut lease (Invariants 3 & 4).
    ref_mut_leases: BTreeSet<NodeId>,
    next_node_id: u64,
    next_edge_id: u64,
}

impl NodeStore {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
            edges_from: BTreeMap::new(),
            edges_to: BTreeMap::new(),
            ref_mut_leases: BTreeSet::new(),
            next_node_id: 1,
            next_edge_id: 1,
        }
    }

    fn alloc_node_id(&mut self) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        id
    }

    fn alloc_edge_id(&mut self) -> EdgeId {
        let id = EdgeId(self.next_edge_id);
        self.next_edge_id += 1;
        id
    }

    // ── Node operations ──────────────────────────────────────────────────────

    /// Create a new node. `owner = None` means unowned (persistent in the real system).
    /// If `owner` is given, an Own edge from owner → new node is automatically created.
    pub fn create_node(&mut self, owner: Option<NodeId>) -> Result<NodeId, LobError> {
        if let Some(owner_id) = owner {
            let n = self.nodes.get(&owner_id).ok_or(LobError::NodeNotFound)?;
            if n.is_tombstone {
                return Err(LobError::NodeIsTombstone);
            }
        }

        let id = self.alloc_node_id();
        self.nodes.insert(id, Node::new(id, owner));

        if let Some(owner_id) = owner {
            let eid = self.alloc_edge_id();
            let edge = Edge { id: eid, from: owner_id, to: id, kind: EdgeKind::Own, attrs: BTreeMap::new() };
            self.edges.insert(eid, edge);
            self.edges_from.entry(owner_id).or_default().push(eid);
            self.edges_to.entry(id).or_default().push(eid);
        }

        Ok(id)
    }

    /// Read a node.
    pub fn get_node(&self, id: NodeId) -> Result<&Node, LobError> {
        self.nodes.get(&id).ok_or(LobError::NodeNotFound)
    }

    /// Mutably access a node's attrs and data.
    ///
    /// **Invariant 8**: requires a valid [`LeaseToken`] whose target matches `id`.
    pub fn get_node_mut(&mut self, id: NodeId, lease: &LeaseToken) -> Result<&mut Node, LobError> {
        if lease.target != id || !self.ref_mut_leases.contains(&id) {
            return Err(LobError::InvalidLease);
        }
        self.nodes.get_mut(&id).ok_or(LobError::NodeNotFound)
    }

    /// Transfer ownership of `node_id` to `new_owner` (or to unowned if `None`).
    ///
    /// **Invariant 2**: fails if any Ref edges point to the node.
    /// **Invariant 3**: fails if a ref_mut lease is active on the node.
    /// **Invariant 5**: fails if the transfer would create an ownership cycle.
    pub fn move_node(&mut self, node_id: NodeId, new_owner: Option<NodeId>) -> Result<(), LobError> {
        {
            let node = self.nodes.get(&node_id).ok_or(LobError::NodeNotFound)?;
            if node.is_tombstone {
                return Err(LobError::NodeIsTombstone);
            }
        }

        // Invariant 3: no active ref_mut lease.
        if self.ref_mut_leases.contains(&node_id) {
            return Err(LobError::LeaseActive);
        }

        // Invariant 2: no incoming Ref edges.
        let incoming_eids: Vec<EdgeId> = self.edges_to.get(&node_id).cloned().unwrap_or_default();
        for eid in &incoming_eids {
            if let Some(e) = self.edges.get(eid) {
                if e.kind == EdgeKind::Ref {
                    return Err(LobError::RefsMustBeReleased);
                }
            }
        }

        // Validate new owner.
        if let Some(new_owner_id) = new_owner {
            let n = self.nodes.get(&new_owner_id).ok_or(LobError::NodeNotFound)?;
            if n.is_tombstone {
                return Err(LobError::NodeIsTombstone);
            }
            // Invariant 5: no new cycle.
            if self.would_create_ownership_cycle(new_owner_id, node_id) {
                return Err(LobError::OwnershipCycle);
            }
        }

        // Remove the old Own edge.
        let old_owner = self.nodes.get(&node_id).unwrap().owner;
        if let Some(old_owner_id) = old_owner {
            let old_eid = incoming_eids.iter().copied().find(|&eid| {
                self.edges.get(&eid).map(|e| e.kind == EdgeKind::Own && e.from == old_owner_id).unwrap_or(false)
            });
            if let Some(eid) = old_eid {
                self.edges.remove(&eid);
                if let Some(v) = self.edges_from.get_mut(&old_owner_id) {
                    v.retain(|&e| e != eid);
                }
                if let Some(v) = self.edges_to.get_mut(&node_id) {
                    v.retain(|&e| e != eid);
                }
            }
        }

        // Set new owner and add Own edge.
        self.nodes.get_mut(&node_id).unwrap().owner = new_owner;
        if let Some(new_owner_id) = new_owner {
            let eid = self.alloc_edge_id();
            let edge = Edge { id: eid, from: new_owner_id, to: node_id, kind: EdgeKind::Own, attrs: BTreeMap::new() };
            self.edges.insert(eid, edge);
            self.edges_from.entry(new_owner_id).or_default().push(eid);
            self.edges_to.entry(node_id).or_default().push(eid);
        }

        Ok(())
    }

    /// Clone a node: new ID, same attrs/data, assigned to `new_owner`.
    pub fn clone_node(&mut self, node_id: NodeId, new_owner: Option<NodeId>) -> Result<NodeId, LobError> {
        let (attrs, data) = {
            let n = self.nodes.get(&node_id).ok_or(LobError::NodeNotFound)?;
            if n.is_tombstone {
                return Err(LobError::NodeIsTombstone);
            }
            (n.attrs.clone(), n.data.clone())
        };
        let new_id = self.create_node(new_owner)?;
        let new_node = self.nodes.get_mut(&new_id).unwrap();
        new_node.attrs = attrs;
        new_node.data = data;
        Ok(new_id)
    }

    /// Drop a node, cascading deletion to all owned descendants.
    ///
    /// **Invariant 6**: all owned nodes are deleted deterministically.
    /// **Invariant 7**: nodes with incoming Weak edges become tombstones rather than
    /// being fully removed; `upgrade()` on their WeakTokens will return `None`.
    ///
    /// Direct drops are blocked if the node has active Ref edges (refcount > 0).
    /// Cascade drops ignore refcount and proceed regardless.
    pub fn drop_node(&mut self, node_id: NodeId) -> Result<(), LobError> {
        let node = self.nodes.get(&node_id).ok_or(LobError::NodeNotFound)?;
        if node.ref_count > 0 {
            return Err(LobError::RefsMustBeReleased);
        }
        self.drop_node_internal(node_id);
        Ok(())
    }

    // ── Edge operations ──────────────────────────────────────────────────────

    /// Add an edge between two existing nodes.
    ///
    /// - **Own**: **Invariant 1** (single owner) and **Invariant 5** (no cycles) are enforced.
    /// - **Ref**: increments the target's `ref_count`.
    /// - **Weak**: no lifetime effect; can point to any node including tombstones.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind) -> Result<EdgeId, LobError> {
        // Validate source.
        {
            let n = self.nodes.get(&from).ok_or(LobError::NodeNotFound)?;
            if n.is_tombstone { return Err(LobError::NodeIsTombstone); }
        }
        // Validate target (Weak may point to tombstones, still must exist).
        self.nodes.get(&to).ok_or(LobError::NodeNotFound)?;
        if kind != EdgeKind::Weak {
            if self.nodes.get(&to).unwrap().is_tombstone {
                return Err(LobError::NodeIsTombstone);
            }
        }

        match kind {
            EdgeKind::Own => {
                // Invariant 1.
                if self.nodes.get(&to).unwrap().owner.is_some() {
                    return Err(LobError::AlreadyOwned);
                }
                // Invariant 5.
                if self.would_create_ownership_cycle(from, to) {
                    return Err(LobError::OwnershipCycle);
                }
                let eid = self.alloc_edge_id();
                let edge = Edge { id: eid, from, to, kind, attrs: BTreeMap::new() };
                self.edges.insert(eid, edge);
                self.edges_from.entry(from).or_default().push(eid);
                self.edges_to.entry(to).or_default().push(eid);
                self.nodes.get_mut(&to).unwrap().owner = Some(from);
                Ok(eid)
            }
            EdgeKind::Ref => {
                let eid = self.alloc_edge_id();
                let edge = Edge { id: eid, from, to, kind, attrs: BTreeMap::new() };
                self.edges.insert(eid, edge);
                self.edges_from.entry(from).or_default().push(eid);
                self.edges_to.entry(to).or_default().push(eid);
                self.nodes.get_mut(&to).unwrap().ref_count += 1;
                Ok(eid)
            }
            EdgeKind::Weak => {
                let eid = self.alloc_edge_id();
                let edge = Edge { id: eid, from, to, kind, attrs: BTreeMap::new() };
                self.edges.insert(eid, edge);
                self.edges_from.entry(from).or_default().push(eid);
                self.edges_to.entry(to).or_default().push(eid);
                Ok(eid)
            }
        }
    }

    /// Remove a Ref or Weak edge by ID. Decrements `ref_count` for Ref edges.
    pub fn remove_edge(&mut self, edge_id: EdgeId) -> Result<(), LobError> {
        let edge = self.edges.remove(&edge_id).ok_or(LobError::EdgeNotFound)?;
        if let Some(v) = self.edges_from.get_mut(&edge.from) { v.retain(|&e| e != edge_id); }
        if let Some(v) = self.edges_to.get_mut(&edge.to)   { v.retain(|&e| e != edge_id); }
        if edge.kind == EdgeKind::Ref {
            if let Some(n) = self.nodes.get_mut(&edge.to) {
                n.ref_count = n.ref_count.saturating_sub(1);
            }
        }
        Ok(())
    }

    // ── Lease / borrow operations ─────────────────────────────────────────────

    /// Acquire an exclusive write lease on a node.
    ///
    /// **Invariant 4**: only one ref_mut lease per node at a time.
    /// Returns a [`LeaseToken`] required by [`NodeStore::get_node_mut`].
    pub fn ref_mut(&mut self, node_id: NodeId) -> Result<LeaseToken, LobError> {
        let n = self.nodes.get(&node_id).ok_or(LobError::NodeNotFound)?;
        if n.is_tombstone { return Err(LobError::NodeIsTombstone); }
        if self.ref_mut_leases.contains(&node_id) {
            return Err(LobError::LeaseActive);
        }
        self.ref_mut_leases.insert(node_id);
        Ok(LeaseToken { target: node_id })
    }

    /// Release a ref_mut lease.
    pub fn release_ref_mut(&mut self, token: LeaseToken) {
        self.ref_mut_leases.remove(&token.target);
    }

    /// Acquire a shared borrow (Ref edge). Multiple can coexist. Prevents movement of target.
    pub fn ref_borrow(&mut self, from: NodeId, to: NodeId) -> Result<RefToken, LobError> {
        let eid = self.add_edge(from, to, EdgeKind::Ref)?;
        Ok(RefToken { edge_id: eid, target: to })
    }

    /// Release a shared borrow.
    ///
    /// Returns `NodeNotFound` if the target node was cascade-deleted while the Ref was held.
    pub fn release_ref(&mut self, token: RefToken) -> Result<(), LobError> {
        // Check if target still exists and is not a tombstone
        match self.nodes.get(&token.target) {
            None => return Err(LobError::NodeNotFound),
            Some(n) if n.is_tombstone => return Err(LobError::NodeNotFound),
            _ => {}
        }
        self.remove_edge(token.edge_id)
    }

    /// Create a Weak edge (no lifetime effect).
    pub fn weak(&mut self, from: NodeId, to: NodeId) -> Result<WeakToken, LobError> {
        let eid = self.add_edge(from, to, EdgeKind::Weak)?;
        Ok(WeakToken { edge_id: eid, target: to })
    }

    /// Attempt to upgrade a Weak edge to a shared [`RefToken`].
    ///
    /// **Invariant 7**: returns `Ok(None)` if the target is a tombstone or has been
    /// fully removed — Weak edges never prevent deletion.
    pub fn upgrade(&mut self, token: &WeakToken) -> Result<Option<RefToken>, LobError> {
        let (from, to) = {
            let edge = self.edges.get(&token.edge_id).ok_or(LobError::EdgeNotFound)?;
            (edge.from, edge.to)
        };
        match self.nodes.get(&to) {
            None => return Ok(None),
            Some(n) if n.is_tombstone => return Ok(None),
            _ => {}
        }
        let eid = self.alloc_edge_id();
        let edge = Edge { id: eid, from, to, kind: EdgeKind::Ref, attrs: BTreeMap::new() };
        self.edges.insert(eid, edge);
        self.edges_from.entry(from).or_default().push(eid);
        self.edges_to.entry(to).or_default().push(eid);
        self.nodes.get_mut(&to).unwrap().ref_count += 1;
        Ok(Some(RefToken { edge_id: eid, target: to }))
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Recursively drop a node and all its owned descendants (Invariant 6).
    /// Nodes with remaining incoming Weak edges become tombstones (Invariant 7).
    fn drop_node_internal(&mut self, node_id: NodeId) {
        // Collect owned children before mutating.
        let owned_children: Vec<NodeId> = self.edges_from
            .get(&node_id)
            .into_iter()
            .flat_map(|ids| ids.iter().copied())
            .filter_map(|eid| {
                self.edges.get(&eid).and_then(|e| {
                    if e.kind == EdgeKind::Own { Some(e.to) } else { None }
                })
            })
            .collect();

        for child in owned_children {
            self.drop_node_internal(child);
        }

        // Release any active lease (process died mid-operation).
        self.ref_mut_leases.remove(&node_id);

        // Remove all outgoing edges.
        let outgoing: Vec<EdgeId> = self.edges_from.remove(&node_id).unwrap_or_default();
        for eid in outgoing {
            if let Some(edge) = self.edges.remove(&eid) {
                if let Some(v) = self.edges_to.get_mut(&edge.to) { v.retain(|&e| e != eid); }
                if edge.kind == EdgeKind::Ref {
                    if let Some(n) = self.nodes.get_mut(&edge.to) {
                        n.ref_count = n.ref_count.saturating_sub(1);
                    }
                }
            }
        }

        // Collect incoming edge info before mutating edges map.
        let incoming: Vec<EdgeId> = self.edges_to.remove(&node_id).unwrap_or_default();
        let incoming_info: Vec<(EdgeId, NodeId, EdgeKind)> = incoming.iter()
            .filter_map(|&eid| self.edges.get(&eid).map(|e| (eid, e.from, e.kind)))
            .collect();

        // Remove non-weak incoming edges from edges_from of their sources.
        let mut weak_incoming: Vec<EdgeId> = Vec::new();
        for (eid, from, kind) in &incoming_info {
            if *kind == EdgeKind::Weak {
                weak_incoming.push(*eid);
            } else {
                if let Some(v) = self.edges_from.get_mut(from) { v.retain(|&e| e != *eid); }
                self.edges.remove(eid);
            }
        }

        if weak_incoming.is_empty() {
            self.nodes.remove(&node_id);
        } else {
            // Tombstone: clear data but keep shell for upgrade() checks.
            let node = self.nodes.get_mut(&node_id).unwrap();
            node.is_tombstone = true;
            node.owner = None;
            node.ref_count = 0;
            node.data = None;
            node.attrs.clear();
            self.edges_to.insert(node_id, weak_incoming);
        }
    }

    // ── Traversal ─────────────────────────────────────────────────────────────

    /// Walk the graph from `root` along edges of the given kinds, up to `max_depth` hops.
    ///
    /// - `direction`: `Forward` follows outgoing edges, `Reverse` follows incoming edges,
    ///   `Both` follows both simultaneously.
    /// - `include_root`: whether to include `root` itself in the returned set.
    ///
    /// Returns nodes in DFS visitation order. Note: DFS order is stack-reversal order
    /// (children visited right-to-left as pushed) and is not guaranteed stable across
    /// mutations. If BFS / nearest-first ordering is needed, use a queue instead.
    ///
    /// Tombstone nodes are included if reachable and not filtered by the caller.
    /// This is intentional: a tombstone is a node whose owner was dropped but which
    /// still has incoming Weak edges. Reverse Weak traversal will naturally encounter
    /// them, and silently hiding them would discard exactly the information a forensics
    /// or provenance query is looking for — that something *used to exist here*.
    ///
    /// The pre-check `!visited.contains` before pushing is a space/redundancy tradeoff:
    /// it avoids stack bloat on dense graphs with many cross-edges, at the cost of a
    /// redundant lookup (the `visited.insert` at pop time is the authoritative guard).
    ///
    /// Returns `Err(NodeNotFound)` if `root` does not exist, so callers can distinguish
    /// "nothing reachable" from "root was invalid."
    pub fn traverse(
        &self,
        root: NodeId,
        edge_kinds: &[EdgeKind],
        max_depth: usize,
        direction: Direction,
        include_root: bool,
    ) -> Result<Vec<NodeId>, LobError> {
        self.nodes.get(&root).ok_or(LobError::NodeNotFound)?;

        let mut visited: BTreeSet<NodeId> = BTreeSet::new();
        let mut results: Vec<NodeId> = Vec::new();
        let mut stack: Vec<(NodeId, usize)> = alloc::vec![(root, 0)];

        while let Some((current, depth)) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if current != root || include_root {
                results.push(current);
            }
            if depth >= max_depth {
                continue;
            }
            if matches!(direction, Direction::Forward | Direction::Both) {
                if let Some(eids) = self.edges_from.get(&current) {
                    for &eid in eids {
                        if let Some(e) = self.edges.get(&eid) {
                            if edge_kinds.contains(&e.kind) && !visited.contains(&e.to) {
                                stack.push((e.to, depth + 1));
                            }
                        }
                    }
                }
            }
            if matches!(direction, Direction::Reverse | Direction::Both) {
                if let Some(eids) = self.edges_to.get(&current) {
                    for &eid in eids {
                        if let Some(e) = self.edges.get(&eid) {
                            if edge_kinds.contains(&e.kind) && !visited.contains(&e.from) {
                                stack.push((e.from, depth + 1));
                            }
                        }
                    }
                }
            }
        }
        Ok(results)
    }

    /// Returns true if adding an Own edge `parent → child` would create a cycle.
    /// Performs a DFS from `child` following existing Own edges, looking for `parent`.
    fn would_create_ownership_cycle(&self, parent: NodeId, child: NodeId) -> bool {
        let mut visited: BTreeSet<NodeId> = BTreeSet::new();
        let mut stack = alloc::vec![child];
        while let Some(current) = stack.pop() {
            if current == parent {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(eids) = self.edges_from.get(&current) {
                for &eid in eids {
                    if let Some(e) = self.edges.get(&eid) {
                        if e.kind == EdgeKind::Own {
                            stack.push(e.to);
                        }
                    }
                }
            }
        }
        false
    }
}

impl Default for NodeStore {
    fn default() -> Self {
        Self::new()
    }
}
