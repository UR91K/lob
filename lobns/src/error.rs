/// All errors the node store can return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobError {
    /// Referenced node does not exist.
    NodeNotFound,
    /// Referenced edge does not exist.
    EdgeNotFound,
    /// Invariant 1: target already has an owner; cannot add a second Own edge.
    AlreadyOwned,
    /// Invariant 2: Ref edges pointing to the node must be released before moving it.
    RefsMustBeReleased,
    /// Invariants 3 & 4: a ref_mut lease is already active on this node.
    LeaseActive,
    /// Invariant 5: adding this Own edge would create an ownership cycle.
    OwnershipCycle,
    /// Invariant 8: the provided LeaseToken does not match the target node.
    InvalidLease,
    /// Operation attempted on a tombstone node (dropped but weak refs remain).
    NodeIsTombstone,
}
