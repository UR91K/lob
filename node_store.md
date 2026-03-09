# LOBNS — The LOB Node Store

LOBNS is the core data structure of LOB. It replaces the traditional filesystem with a graph of nodes connected by edges. This document describes the node and edge definitions, syscall API, memory model, and journaling rules.

---

## Node Definition

Every entity in the system is a node. The struct is split into two clearly separated namespaces: fields the kernel owns and enforces, and fields the application owns.

```rust
pub struct Node {
    // Kernel-enforced — application cannot set or modify these
    // The kernel stamps and maintains these fields exclusively
    pub id:                  NodeId,          // u64, unique, never reused, never zero
    pub owner:               Option<NodeId>,  // None means unowned — persists indefinitely
    pub refcount:            u32,             // number of active Ref edges pointing here
    pub ref_mut:             bool,            // is an exclusive write lease active?
    pub content_hash:        [u8; 32],        // BLAKE3, updated by kernel on every write
    pub created_at:          u64,             // unix timestamp, set once at creation
    pub modified_at:         u64,             // updated by kernel on every ref_mut write
    pub accessed_at:         u64,             // updated by kernel on every ref
    pub created_by_process:  Option<NodeId>,  // weak — specific process instance, may tombstone
    pub created_by_binary:   Option<NodeId>,  // weak — the binary node, survives process death
    pub created_by_session:  u64,             // session ID scalar, recorded at creation
    pub created_by_user:     UserId,          // never None, always stamped from session context
    pub signature:           Option<[u8; 64]>, // Ed25519 signature over content_hash
    pub signing_key:         Option<[u8; 32]>, // public key of signer

    // Application-owned — kernel is completely indifferent to these
    pub edges:  Vec<Edge>,
    pub attrs:  BTreeMap<String, Value>,
    pub data:   Option<Vec<u8>>,
}
```

The separation between kernel fields and application fields is enforced at the syscall boundary. Kernel fields do not exist in the attr namespace — there is no string key that aliases to a kernel field, no way to shadow or spoof kernel-stamped data from userspace.

---

## Edge Definition

```rust
pub struct Edge {
    // Kernel-enforced — the only two fields the kernel ever inspects
    pub kind:   EdgeKind,  // determines lifetime semantics
    pub target: NodeId,    // kernel validates this node exists at edge creation

    // Application-owned — kernel is completely indifferent
    pub attrs:  BTreeMap<String, Value>,  // label, metadata, anything the application wants
}

pub enum EdgeKind {
    Own,   // at most one per target node, cascade deletion, no cycles permitted
    Ref,   // increments target refcount, prevents deletion while held
    Weak,  // tombstoned on target deletion, no lifetime effect
}

pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
    Bytes(Vec<u8>),
    NodeRef(NodeId),  // a reference to another node stored as a value
}
```

---

## The Three Edge Types

| Edge | Semantics | Lifetime effect |
|---|---|---|
| `Own` | One per node maximum, target dies when dropped | Determines lifetime |
| `Ref` | Many allowed, shared borrow, read semantics | Keeps target alive |
| `Weak` | Provenance and backlinks, no lifetime effect | None — target may become tombstone |

Because ownership edges must form a DAG (verified at creation time), ownership cycles are structurally impossible. Ref cycles can exist but do not cause leaks because only Own edges determine node lifetime. This is the same reason `Weak<T>` breaks cycles in Rust.

---

## The LOBNS Syscall API

The LOBNS syscalls use Rust semantics directly. This is not merely inspired by Rust — it is the actual ownership model, implemented as kernel syscalls operating on persistent graph nodes instead of memory.

### Node Operations

```rust
move(node, to)      // transfer ownership, exactly like Rust's move
clone(node, to)     // duplicate a node, new ID, new owner
ref(node)           // create a Ref edge, borrow
ref_mut(node)       // create exclusive write lease, &mut
drop(node)          // explicitly release ownership, triggers cascade
weak(node)          // create a Weak edge, no lifetime implication
upgrade(weak_ref)   // attempt to promote Weak to Ref, fails if node is tombstone
```

### Anonymous Memory Operations

For performance-critical code and temporary data, processes have access to traditional anonymous memory:

```rust
mmap(size, flags)   // allocate anonymous memory region
munmap(ptr, size)   // deallocate memory region
brk(addr)           // adjust heap boundary
```

Anonymous memory is fast, lightweight, process-local, and dies with the process. It is not visible in the node graph and has no provenance tracking. Use it for hot paths, temporary buffers, and data structures that don't need persistence.

---

## Memory Model and Journaling

LOB provides three tiers of memory for different use cases:

### Anonymous Memory

Traditional stack and heap allocation (`malloc`, `brk`, `mmap`) for:
- Performance-critical code and hot paths
- Temporary buffers and scratch space
- Data structures that never outlive the process
- Computation that doesn't need persistence or provenance

Anonymous memory is fast, lightweight, invisible in the node graph, and dies with the process. No syscall overhead after initial allocation.

### Nodes Owned by Ephemeral Entities

Nodes owned by processes or sessions:
- Tracked with full provenance (created_by_binary, created_by_user, etc.)
- Visible in the node graph for debugging
- **Not journaled** — live only in RAM
- Die when the owning process/session exits
- Can be shared between processes via IPC
- Can be promoted to persistent by moving to UNOWNED

### Nodes Owned by Persistent Entities or Unowned

Nodes owned by unowned nodes (like packages) or unowned themselves:
- Full provenance tracking
- Visible and queryable in the node graph
- **Journaled to disk** — survive reboots
- Persist indefinitely until explicitly dropped
- Content-addressed with BLAKE3 hashes

---

## Journaling Rules

The kernel determines whether to journal a node by walking up its ownership chain:

```rust
fn should_journal(node: NodeId) -> bool {
    let mut current = node;
    loop {
        match store.get_node(current).owner {
            None => return true,  // unowned → journal
            Some(owner) => {
                if is_ephemeral_root(owner) {
                    return false;  // owned by process/session → no journal
                }
                current = owner;  // keep walking up
            }
        }
    }
}
```

**Key insight:** Ownership determines lifetime semantics (cascade deletion). Root ownership determines persistence (journaling). These are orthogonal properties.

**Examples:**

```rust
// Owned by process → ephemeral, not journaled
[process] ──own──► [scratch buffer]

// Owned by unowned package → persistent, journaled
[package: firefox] ──own──► [binary: firefox]
(package.owner = None)

// Unowned document owns revisions → all journaled
[document] ──own──► [revision 1]
           ──own──► [revision 2]
(document.owner = None)
```

When a node is moved to UNOWNED, the kernel computes its content_hash (if not already computed) and writes it to the journal. The node becomes persistent at that moment.

---

## Swap and Paging

**Nodes are never paged.** Nodes owned by ephemeral entities stay in RAM. Persistent nodes live in the node store on disk and are cached in RAM when accessed. The kernel never pages node data under memory pressure.

**Anonymous memory can be paged** if swap is configured. Swap is optional, similar to Linux:

- **With swap configured:** The kernel can page anonymous memory to swap under memory pressure. Swap can be a raw partition or a special unowned node with `type:swap`. Anonymous memory allocations can overcommit.

- **Without swap:** Anonymous memory stays in RAM. Allocations fail when RAM is exhausted. The system runs entirely without paging.

This separation keeps node semantics clean (ownership and journaling rules are never affected by memory pressure) while giving administrators flexibility in how they manage anonymous memory.

---

## Persistence — Unowned by Default

Nodes in LOBNS are **persistent by default** if they are unowned or owned by persistent entities. An unowned node exists indefinitely. It survives reboots, process deaths, and session ends. Nothing needs to explicitly mark a node as persistent — being unowned (or owned by an unowned node) makes it persistent.

Ephemerality is the exception, not the rule. A node is ephemeral only when it is explicitly owned by an ephemeral entity — a process or session. When the owner is dropped, the cascade reaches the owned nodes and they disappear automatically. No flags, no tiers, no kernel heuristics.

**Saving a document is just releasing ownership:**

```rust
// Buffer is owned by the editor process — ephemeral, not journaled
let buffer = liblob::create()
    .attr("type", "text")
    .owner(self_process_id)
    .build()?;

// Save — move to unowned, node becomes persistent and journaled
liblob::move(buffer, UNOWNED)?;
// Now survives process death, session end, and reboots
// Found by query, not by location
```

`move(node, UNOWNED)` is the commit point. Before it, work is transactional — process death rolls it back automatically because the node is not journaled. After it, the node is journaled, independent, and permanent.

---

## The Two Graphs — Memory and Disk

LOBNS presents one unified store to applications. Internally the graph has two very different shapes.

### In Memory — One Ownership Tree

Everything currently running is part of a single ownership tree rooted at the init process. This tree exists only for the duration of the session. Its sole purpose is deterministic cleanup:

```
[init process]
  └──own──► [session]
                ├──own──► [process: browser]
                │              ├──own──► [tab: gmail]       ← ephemeral
                │              └──own──► [scratch buffer]   ← ephemeral
                │
                └──own──► [process: editor]
                               └──own──► [unsaved buffer]   ← ephemeral
```

When the session ends, the session node is dropped and everything in this tree cascades away — scratch buffers, unsaved work, open handles. Nothing in this tree needs explicit cleanup code.

### On Disk — Flat Islands

On disk there is no grand ownership tree. There are free-floating unowned nodes connected by Ref and Weak edges, with small owned islands only where cascade deletion is genuinely the right semantic behaviour:

```
Free-floating unowned nodes (the vast majority):

[firefox binary]    type:executable  package:firefox
[firefox profile]   type:app_data    app:firefox      user:alice
[notes.txt]         type:text        user:alice        tag:work
[network config]    type:config      scope:system
[bookmark: github]  type:bookmark    app:firefox       user:alice

Small owned islands (packages, revision histories):

[pkg: firefox] ──own──► [bin: firefox]
               ──own──► [bin: crashreporter]
               ──own──► [res: icons]

[notes.txt] ──own──► [revision: v1]
            ──own──► [revision: v2]
```

The firefox profile is not owned by the firefox package. It has `app:firefox` as an attribute and a weak backlink to the binary, but no ownership edge. Uninstalling firefox drops the package node and cascades to its owned binaries and resources. The profile survives because it was never owned by the package — it simply has a relationship with it, which is the correct semantic.

The rule on disk: **own only where cascade deletion is genuinely the right behaviour.** Everything else is free-floating, found by query, connected by Ref and Weak edges that express relationships without imposing lifetime constraints.

---

## Ref and Weak Edges Are the Connective Tissue

On disk, Ref and Weak edges express relationships and record history. They are what connects the flat islands into a queryable semantic graph:

```
[firefox profile] ──weak──► [firefox binary]     "created by this program"
[bookmark: github] ──ref──► [firefox profile]    "belongs to this profile"
[download: cat.jpg] ──ref──► [bookmark: github]  "downloaded from this page"
[notes.txt v2] ──weak──► [notes.txt v1]          "previous revision"
```

None of these edges affect lifetime. They are the permanent record of how everything in the system relates to everything else — queryable at any time, traversable in the graph view.

---

## Provenance Is Unforgeable

`created_by_process`, `created_by_binary`, and `created_by_user` are stamped by the kernel at node creation time from the current execution context. A process cannot claim its nodes were created by a different binary or user. A compromised process cannot retroactively attribute its actions elsewhere. Provenance is a kernel guarantee, not a convention.

`created_by_process` is a weak link to the specific running instance — it becomes a tombstone when the process exits, which is expected and normal. `created_by_binary` is a weak link to the binary node on disk, which persists long after the process is gone and remains permanently resolvable. The binary node ID is therefore the stable long-term identity for provenance queries.

`created_by_user` is never None. Every node is created in some execution context — UserId(0) for kernel-created nodes, UserId(1) for init, real user IDs from there. User identity is assigned when a session is created and inherited by all processes spawned in that session. A process cannot claim a different UserId any more than it can spoof `created_by_binary`.

```
// Which specific instance? (tombstone after process exits)
query created_by_process:[node-id]

// Which program? (always resolvable, binary still on disk)
query created_by_binary:[node-id]

// Everything firefox has ever created, all instances, all time
query created_by_binary:[firefox-binary-node-id]

// Everything alice has ever created
query created_by_user:alice
```

---

## The Journal

The journal sits between the in-memory node store and raw storage. Its sole responsibility is ensuring every node write is atomic — a partial write caused by power loss leaves the system in a recoverable state.

The journal is intentionally as simple as possible: a linear append-only log with a single commit record, similar to SQLite's WAL. Every mutation is written to the journal first. A commit record is written only when the mutation is complete. On boot, the kernel replays committed but not yet checkpointed entries and discards uncommitted ones. After recovery, all eight invariants are verified.

Simplicity is a correctness strategy. The simpler the journal, the more obviously correct it is, and the easier it is to test with fault injection.

---

## Names Are Artificial

In a conventional filesystem, a filename is fundamental — it is how the filesystem locates a node. In LOBNS, identity is the node ID. A name is just an attribute, no different from `type` or `created_at`. There is nothing structurally special about it.

Nameless nodes are the natural default, not a special case. A process that needs scratch space creates a nameless node it owns. When the process exits, the cascade reaches the scratch node and it disappears. No name to invent, no `/tmp` collision, no cleanup code required.

The vocabulary deliberately separates itself from POSIX conventions. There are no files, no directories, no paths, no mounts. There are only nodes, edges, and queries:

| POSIX | LOB |
|---|---|
| file | node |
| directory | view or owned subgraph |
| path | node ID or query |
| file descriptor | ref handle |
| inode | node |
| symlink | — not needed |
| hardlink | — not needed |
| filesystem | LOBNS |

---

## Application Design Considerations

Designing applications on LOB means choosing the right memory tier:

**Use anonymous memory when:**
- Performance is critical
- Data is truly temporary (function-local, loop iteration)
- No need for crash recovery or provenance

**Use owned nodes when:**
- Working state should survive crashes
- You want visibility in the graph for debugging
- Data might be shared with other processes
- You want provenance tracking

**Use unowned nodes when:**
- User documents and persistent data
- Configuration and preferences
- Audit logs and history
- Anything that should survive reboots

---

See also:
- [README.md](README.md) — Overview and quick start
- [security.md](security.md) — Access control and provenance
- [user_experience.md](user_experience.md) — Query language and browser
- [implementation.md](implementation.md) — Testing and development phases
