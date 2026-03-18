/// Invariant tests for the LOB node store.
///
/// Each test is labelled with the invariant(s) it covers:
///
/// 1. Every node has exactly one owner, or is explicitly unowned.
/// 2. You cannot move a node while any Ref edge points to it.
/// 3. You cannot move a node while any ref_mut lease is active on it.
/// 4. Only one ref_mut lease can exist for a node at any time.
/// 5. Ownership edges cannot form cycles.
/// 6. Dropping an owner cascades to all owned nodes deterministically.
/// 7. Weak edges never prevent deletion; upgrade() can return None.
/// 8. A node's data cannot be mutated without a ref_mut lease.
use lobns::{Direction, EdgeKind, LobError, NodeId, NodeStore};

// ── Invariant 8 ──────────────────────────────────────────────────────────────

#[test]
fn inv1_node_can_be_unowned() {
    let mut store = NodeStore::new();
    let node = store.create_node(None).unwrap();
    assert_eq!(store.get_node(node).unwrap().owner, None);
}

#[test]
fn inv1_node_gets_owner_on_create() {
    let mut store = NodeStore::new();
    let owner = store.create_node(None).unwrap();
    let child = store.create_node(Some(owner)).unwrap();
    assert_eq!(store.get_node(child).unwrap().owner, Some(owner));
}

#[test]
fn inv1_second_own_edge_rejected() {
    let mut store = NodeStore::new();
    let owner1 = store.create_node(None).unwrap();
    let owner2 = store.create_node(None).unwrap();
    let child = store.create_node(Some(owner1)).unwrap();
    // Child already has owner1; attempting to give it owner2 must fail.
    assert_eq!(store.add_edge(owner2, child, EdgeKind::Own), Err(LobError::AlreadyOwned));
}

// ── Invariant 2 ──────────────────────────────────────────────────────────────

#[test]
fn inv2_move_blocked_by_ref_edge() {
    let mut store = NodeStore::new();
    let borrower = store.create_node(None).unwrap();
    let target   = store.create_node(None).unwrap();
    let new_owner = store.create_node(None).unwrap();

    let _tok = store.ref_borrow(borrower, target).unwrap();
    assert_eq!(store.move_node(target, Some(new_owner)), Err(LobError::RefsMustBeReleased));
}

#[test]
fn inv2_move_allowed_after_ref_released() {
    let mut store = NodeStore::new();
    let borrower  = store.create_node(None).unwrap();
    let target    = store.create_node(None).unwrap();
    let new_owner = store.create_node(None).unwrap();

    let tok = store.ref_borrow(borrower, target).unwrap();
    store.release_ref(tok).unwrap();
    assert!(store.move_node(target, Some(new_owner)).is_ok());
}

// inv2: move blocked by *multiple* refs, allowed only after *all* released
#[test]
fn inv2_move_blocked_until_all_refs_released() {
    let mut store = NodeStore::new();
    let b1 = store.create_node(None).unwrap();
    let b2 = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();
    let new_owner = store.create_node(None).unwrap();

    let tok1 = store.ref_borrow(b1, target).unwrap();
    let tok2 = store.ref_borrow(b2, target).unwrap();
    store.release_ref(tok1).unwrap();
    // Still one ref held — move must still fail
    assert_eq!(store.move_node(target, Some(new_owner)), Err(LobError::RefsMustBeReleased));
    store.release_ref(tok2).unwrap();
    assert!(store.move_node(target, Some(new_owner)).is_ok());
}

// ── Invariant 3 ──────────────────────────────────────────────────────────────

#[test]
fn inv3_move_blocked_by_active_lease() {
    let mut store = NodeStore::new();
    let target    = store.create_node(None).unwrap();
    let new_owner = store.create_node(None).unwrap();

    let _lease = store.ref_mut(target).unwrap();
    assert_eq!(store.move_node(target, Some(new_owner)), Err(LobError::LeaseActive));
}

#[test]
fn inv3_move_allowed_after_lease_released() {
    let mut store = NodeStore::new();
    let target    = store.create_node(None).unwrap();
    let new_owner = store.create_node(None).unwrap();

    let lease = store.ref_mut(target).unwrap();
    store.release_ref_mut(lease);
    assert!(store.move_node(target, Some(new_owner)).is_ok());
}

// ── Invariant 4 ──────────────────────────────────────────────────────────────

#[test]
fn inv4_second_ref_mut_rejected() {
    let mut store = NodeStore::new();
    let target = store.create_node(None).unwrap();

    let _lease1 = store.ref_mut(target).unwrap();
    assert_eq!(store.ref_mut(target), Err(LobError::LeaseActive));
}

#[test]
fn inv4_ref_mut_acquirable_after_release() {
    let mut store = NodeStore::new();
    let target = store.create_node(None).unwrap();

    let lease = store.ref_mut(target).unwrap();
    store.release_ref_mut(lease);
    assert!(store.ref_mut(target).is_ok());
}

// ── Invariant 5 ──────────────────────────────────────────────────────────────

#[test]
fn inv5_direct_ownership_cycle_rejected() {
    let mut store = NodeStore::new();
    let a = store.create_node(None).unwrap();
    let b = store.create_node(Some(a)).unwrap(); // a → b

    // b → a would form a cycle.
    assert_eq!(store.add_edge(b, a, EdgeKind::Own), Err(LobError::OwnershipCycle));
}

#[test]
fn inv5_indirect_ownership_cycle_rejected() {
    let mut store = NodeStore::new();
    let a = store.create_node(None).unwrap();
    let b = store.create_node(Some(a)).unwrap(); // a → b
    let c = store.create_node(Some(b)).unwrap(); // b → c

    // c → a would form the cycle a → b → c → a.
    assert_eq!(store.add_edge(c, a, EdgeKind::Own), Err(LobError::OwnershipCycle));
}

#[test]
fn inv5_self_loop_rejected() {
    let mut store = NodeStore::new();
    let a = store.create_node(None).unwrap();
    assert_eq!(store.add_edge(a, a, EdgeKind::Own), Err(LobError::OwnershipCycle));
}

// inv5: Ref and Weak cycles are allowed (only Own cycles are forbidden)
#[test]
fn inv5_ref_cycle_permitted() {
    let mut store = NodeStore::new();
    let a = store.create_node(None).unwrap();
    let b = store.create_node(None).unwrap();
    store.add_edge(a, b, EdgeKind::Ref).unwrap();
    assert!(store.add_edge(b, a, EdgeKind::Ref).is_ok());
}

// ── Invariant 6 ──────────────────────────────────────────────────────────────

#[test]
fn inv6_cascade_drops_all_owned_descendants() {
    let mut store = NodeStore::new();
    let root       = store.create_node(None).unwrap();
    let child1     = store.create_node(Some(root)).unwrap();
    let child2     = store.create_node(Some(root)).unwrap();
    let grandchild = store.create_node(Some(child1)).unwrap();

    store.drop_node(root).unwrap();

    assert_eq!(store.get_node(root).unwrap_err(),       LobError::NodeNotFound);
    assert_eq!(store.get_node(child1).unwrap_err(),     LobError::NodeNotFound);
    assert_eq!(store.get_node(child2).unwrap_err(),     LobError::NodeNotFound);
    assert_eq!(store.get_node(grandchild).unwrap_err(), LobError::NodeNotFound);
}

#[test]
fn inv6_sibling_unaffected_by_partial_drop() {
    let mut store = NodeStore::new();
    let root   = store.create_node(None).unwrap();
    let child1 = store.create_node(Some(root)).unwrap();
    let child2 = store.create_node(Some(root)).unwrap();

    // Move child1 to unowned (detach from root), then drop root.
    store.move_node(child1, None).unwrap();
    store.drop_node(root).unwrap();

    // root and child2 should be gone.
    assert_eq!(store.get_node(root).unwrap_err(),   LobError::NodeNotFound);
    assert_eq!(store.get_node(child2).unwrap_err(), LobError::NodeNotFound);
    // child1 was detached and should still exist.
    assert!(store.get_node(child1).is_ok());
}

// inv6: cascade with weak edges — weak holders get tombstones, not errors
#[test]
fn inv6_cascade_tombstones_weak_holders() {
    let mut store = NodeStore::new();
    let root = store.create_node(None).unwrap();
    let child = store.create_node(Some(root)).unwrap();
    let holder = store.create_node(None).unwrap();
    let weak_tok = store.weak(holder, child).unwrap();

    store.drop_node(root).unwrap();

    // child should be tombstone, not NodeNotFound
    assert!(store.get_node(child).unwrap().is_tombstone);
    // upgrade should return None
    assert!(store.upgrade(&weak_tok).unwrap().is_none());
}

// ── Invariant 7 ──────────────────────────────────────────────────────────────

#[test]
fn inv7_weak_edge_does_not_prevent_deletion() {
    let mut store = NodeStore::new();
    let holder = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();

    let _w = store.weak(holder, target).unwrap();
    // Drop must succeed even though a Weak edge points to target.
    assert!(store.drop_node(target).is_ok());
}

#[test]
fn inv7_dropped_node_becomes_tombstone() {
    let mut store = NodeStore::new();
    let holder = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();

    let _w = store.weak(holder, target).unwrap();
    store.drop_node(target).unwrap();

    let node = store.get_node(target).unwrap();
    assert!(node.is_tombstone);
}

#[test]
fn inv7_upgrade_returns_none_on_tombstone() {
    let mut store = NodeStore::new();
    let holder = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();

    let weak_tok = store.weak(holder, target).unwrap();
    store.drop_node(target).unwrap();

    let result = store.upgrade(&weak_tok).unwrap();
    assert!(result.is_none());
}

#[test]
fn inv7_upgrade_succeeds_on_live_node() {
    let mut store = NodeStore::new();
    let holder = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();

    let weak_tok = store.weak(holder, target).unwrap();
    let ref_tok = store.upgrade(&weak_tok).unwrap();
    assert!(ref_tok.is_some());

    // ref_count should now be 1.
    assert_eq!(store.get_node(target).unwrap().ref_count, 1);
}

// ── Traversal ─────────────────────────────────────────────────────────────────

#[test]
fn traverse_forward_own_excludes_root() {
    let mut store = NodeStore::new();
    let root   = store.create_node(None).unwrap();
    let child  = store.create_node(Some(root)).unwrap();
    let grand  = store.create_node(Some(child)).unwrap();

    let result = store.traverse(root, &[EdgeKind::Own], 10, Direction::Forward, false).unwrap();
    assert!(!result.contains(&root));
    assert!(result.contains(&child));
    assert!(result.contains(&grand));
}

#[test]
fn traverse_forward_includes_root_when_requested() {
    let mut store = NodeStore::new();
    let root  = store.create_node(None).unwrap();
    let child = store.create_node(Some(root)).unwrap();

    let result = store.traverse(root, &[EdgeKind::Own], 10, Direction::Forward, true).unwrap();
    assert!(result.contains(&root));
    assert!(result.contains(&child));
}

#[test]
fn traverse_depth_limit_respected() {
    let mut store = NodeStore::new();
    let root  = store.create_node(None).unwrap();
    let child = store.create_node(Some(root)).unwrap();
    let grand = store.create_node(Some(child)).unwrap();

    // max_depth=1: only direct children, not grandchildren.
    let result = store.traverse(root, &[EdgeKind::Own], 1, Direction::Forward, false).unwrap();
    assert!(result.contains(&child));
    assert!(!result.contains(&grand));
}

#[test]
fn traverse_reverse_finds_owners() {
    let mut store = NodeStore::new();
    let root  = store.create_node(None).unwrap();
    let child = store.create_node(Some(root)).unwrap();

    // Starting from child, reverse Own traversal should reach root.
    let result = store.traverse(child, &[EdgeKind::Own], 10, Direction::Reverse, false).unwrap();
    assert!(result.contains(&root));
    assert!(!result.contains(&child));
}

#[test]
fn traverse_both_directions_finds_siblings() {
    let mut store = NodeStore::new();
    // root owns child1 and child2; starting from child1, Both should reach root and child2.
    let root   = store.create_node(None).unwrap();
    let child1 = store.create_node(Some(root)).unwrap();
    let child2 = store.create_node(Some(root)).unwrap();

    let result = store.traverse(child1, &[EdgeKind::Own], 10, Direction::Both, false).unwrap();
    assert!(result.contains(&root));
    assert!(result.contains(&child2));
}

#[test]
fn traverse_invalid_root_returns_error() {
    let store = NodeStore::new();
    let nonexistent = NodeId(9999);
    assert_eq!(
        store.traverse(nonexistent, &[EdgeKind::Own], 10, Direction::Forward, false).unwrap_err(),
        LobError::NodeNotFound,
    );
}

#[test]
fn traverse_edge_kind_filter_only_follows_specified_kinds() {
    let mut store = NodeStore::new();
    let root   = store.create_node(None).unwrap();
    let child  = store.create_node(Some(root)).unwrap();
    let other  = store.create_node(None).unwrap();
    store.add_edge(root, other, EdgeKind::Ref).unwrap();

    // Only follow Own edges: should reach child but not other.
    let result = store.traverse(root, &[EdgeKind::Own], 10, Direction::Forward, false).unwrap();
    assert!(result.contains(&child));
    assert!(!result.contains(&other));

    // Only follow Ref edges: should reach other but not child.
    let result = store.traverse(root, &[EdgeKind::Ref], 10, Direction::Forward, false).unwrap();
    assert!(!result.contains(&child));
    assert!(result.contains(&other));
}

#[test]
fn inv8_lease_must_match_target() {
    let mut store = NodeStore::new();
    let node_a = store.create_node(None).unwrap();
    let node_b = store.create_node(None).unwrap();

    let lease_a = store.ref_mut(node_a).unwrap();
    // lease_a is for node_a; using it on node_b must fail.
    assert_eq!(store.get_node_mut(node_b, &lease_a).unwrap_err(), LobError::InvalidLease);

    store.release_ref_mut(lease_a);
}

#[test]
fn inv8_mutation_succeeds_with_valid_lease() {
    let mut store = NodeStore::new();
    let node_id = store.create_node(None).unwrap();

    let lease = store.ref_mut(node_id).unwrap();
    {
        let node = store.get_node_mut(node_id, &lease).unwrap();
        node.data = Some(vec![1, 2, 3]);
    }
    store.release_ref_mut(lease);

    assert_eq!(store.get_node(node_id).unwrap().data, Some(vec![1, 2, 3]));

}

// Traversal on a node with no edges returns empty (not an error)
#[test]
fn traverse_isolated_node_returns_empty() {
    let mut store = NodeStore::new();
    let node = store.create_node(None).unwrap();
    let result = store.traverse(node, &[EdgeKind::Own], 10, Direction::Forward, false).unwrap();
    assert!(result.is_empty());
}

// Tombstone nodes are reachable via Weak traversal
#[test]
fn traverse_reaches_tombstone_via_weak() {
    let mut store = NodeStore::new();
    let holder = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();
    store.add_edge(holder, target, EdgeKind::Weak).unwrap();
    store.drop_node(target).unwrap();

    let result = store.traverse(holder, &[EdgeKind::Weak], 10, Direction::Forward, false).unwrap();
    assert!(result.contains(&target));
    assert!(store.get_node(target).unwrap().is_tombstone);
}

// max_depth=0 returns only root (if include_root) or empty
#[test]
fn traverse_zero_depth_returns_empty_without_root() {
    let mut store = NodeStore::new();
    let root = store.create_node(None).unwrap();
    let _child = store.create_node(Some(root)).unwrap();

    let result = store.traverse(root, &[EdgeKind::Own], 0, Direction::Forward, false).unwrap();
    assert!(result.is_empty());
}

// Ref token becomes invalid after cascade
#[test]
fn ref_token_invalid_after_cascade() {
    let mut store = NodeStore::new();
    let owner = store.create_node(None).unwrap();
    let target = store.create_node(Some(owner)).unwrap();
    let borrower = store.create_node(None).unwrap();

    let tok = store.ref_borrow(borrower, target).unwrap();
    store.drop_node(owner).unwrap(); // cascade reaches target and deletes it

    // target is fully gone (no incoming Weak edges, so no tombstone)
    assert_eq!(store.get_node(target).unwrap_err(), LobError::NodeNotFound);

    // using the stale token should return NodeNotFound since target is gone
    assert_eq!(store.release_ref(tok), Err(LobError::NodeNotFound));
}

// drop blocked while Ref edge is held
#[test]
fn drop_blocked_by_active_ref() {
    let mut store = NodeStore::new();
    let borrower = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();

    let _tok = store.ref_borrow(borrower, target).unwrap();
    assert_eq!(store.drop_node(target), Err(LobError::RefsMustBeReleased));
}

// drop succeeds once all Refs are released
#[test]
fn drop_allowed_after_all_refs_released() {
    let mut store = NodeStore::new();
    let b1 = store.create_node(None).unwrap();
    let b2 = store.create_node(None).unwrap();
    let target = store.create_node(None).unwrap();

    let tok1 = store.ref_borrow(b1, target).unwrap();
    let tok2 = store.ref_borrow(b2, target).unwrap();
    store.release_ref(tok1).unwrap();
    assert_eq!(store.drop_node(target), Err(LobError::RefsMustBeReleased));
    store.release_ref(tok2).unwrap();
    assert!(store.drop_node(target).is_ok());
}