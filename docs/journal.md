---

## Journal

The journal is an append-only record of all mutating operations on the node store. It exists as a graph of nodes, protected by the same ownership and ref rules as everything else, with one kernel-level exception for bootstrapping.

### Structure

```
init (process node, owned by kernel)
  Ref → journal-anchor (unowned, persistent)
    Ref → journal-root (unowned, persistent)
      Own → command-record-N
        Own → journal-entry-1
          Own → journal-entry-2
            Own → journal-entry-3
```

**journal-anchor** and **journal-root** are unowned persistent nodes created at system init. Init holds a Ref to the anchor, the anchor holds a Ref to the root. Neither can be dropped because their refcounts are held by the chain above them. Neither is owned by init, so they are not ephemeral and survive across sessions.

**command-records** are owned by the journal-root and represent a single user command. They store metadata — user, command string, timestamp — and own a linked chain of journal entries representing each individual operation the command performed.

**journal-entries** are owned by their command-record and stored as a linked list in execution order. Undo replays them in reverse.

### The `is_journal_entry` Flag

The `Node` struct has one additional kernel-enforced field:

```rust
pub is_journal_entry: bool,
// Set by the kernel at creation time, never modified afterward.
// Nodes with this flag set are exempt from normal syscall paths:
//   - drop() syscall is rejected
//   - ref_mut() syscall is rejected  
//   - The node's own creation is not journaled
// Journal nodes are only modified through the kernel's internal
// journal management path, which undo() uses directly.
```

This flag is set only by the kernel's internal journal writer. It is not an attribute and cannot be set or read through the attribute system. It is surfaced in `show` output as a kernel field alongside `refcount`, `owner` etc.

### Protection Model

The journal cannot be tampered with from userspace:

- **journal-anchor** and **journal-root** cannot be dropped — their refcounts are maintained by the chain from init
- **command-records** and **journal-entries** cannot be dropped or edited — `is_journal_entry` causes the kernel to reject all mutating syscalls on these nodes
- No node can forge `is_journal_entry` — it is kernel-set and lives outside the attribute system

### Bootstrapping Exception

Journal entry nodes are the one case where a node's creation is not itself journaled. This is a necessary kernel-level exception to avoid infinite regression. All other node operations — including creation of command-record nodes and modification of the journal-root's owned children — are written by the kernel's internal journal path directly, bypassing the normal syscall layer.

### Querying the Journal

Because journal nodes are regular graph nodes, they are queryable through normal shell commands:

```shell
# show all command records from today
alice (lobbox1) >> qr -o @journal-root -u alice -c 1d
1 drop-op  | journal-command | cr 2h
2 attr-op  | journal-command | cr 4h
3 clone-op | journal-command | cr 6h

# inspect a specific command record
alice (lobbox1) >> show 1
Node ID: 48291
is_journal_entry: true
Created: 2024-03-11 14:23:11 by lob-shell (9286)
...
Attributes:
  command: "drop @7819"
  user: alice
  entry_count: 47

# attempting to modify a journal node
alice (lobbox1) >> drop 1
Error: Cannot modify journal node 48291
```

### Undo

`undo` is a shell command that calls the kernel's journal management path directly. It finds the most recent command-record owned by the current user, replays its journal entries in reverse order, then drops the command-record node.

```shell
alice (lobbox1) >> undo
Undo 'drop @7819' (47 operations)? [y/N] y
Restored 47 nodes

alice (lobbox1) >> undo --history
1 drop @7819   | 47 ops | 2h ago
2 attr archived:true | 12 ops | 4h ago
3 clone 1 @9281 | 1 op  | 6h ago
```