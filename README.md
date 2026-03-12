# LOB Operating System

LOB (Lifetime, Own, Borrow) is an operating system built around a single insight: the problems that make filesystems unreliable are the same problems that make memory unsafe, and they have the same solution.

Instead of files and directories, LOB manages **nodes**. The node is a uniform primitives that span memory and disk transparently. Whether a node is currently in RAM or on disk is an implementation detail, not a semantic distinction. Nodes are connected by typed edges that encode ownership, reference, and provenance relationships. This graph - memory and storage unified - is the system. There is no separate filesystem, process table, or package database. Everything is a node.

---

## The Problem With Conventional Filesystems

The aliasing-mutation problem is well understood in programming language theory: you cannot safely have both multiple references to a resource and the ability to mutate it simultaneously. Modern type systems solve this structurally by making invalid states unrepresentable, instead of just unlikely.

Filesystems have always had the same problem and have never formally solved it. Instead they accumulate reactive measures:

- **File locking** - advisory, often ignored
- **Permissions** - coarse, do not express aliasing
- **Journaling** - repairs corruption after the fact
- **fsck** - detects invariant violations after they have already happened

These mechanisms detect or repair violations, while LOBNS prevents them from happening in the first place. The kernel enforces ownership and borrowing invariants at every syscall boundary, which makes an invalid graph state is structurally unreachable, not just unlikely.

---

## Core Concepts

### Nodes and Edges

Every entity in the system - running processes, installed packages, user documents, application data, configuration, hardware devices - is a node. Nodes are connected by edges that express relationships and determine lifetime semantics.

### The Three Edge Types

| Edge | Semantics | Lifetime effect |
|---|---|---|
| `Own` | One per node maximum, target dies when dropped | Determines lifetime |
| `Ref` | Many allowed, shared borrow, read semantics | Keeps target alive |
| `Weak` | Provenance and backlinks, no lifetime effect | None - target may become tombstone |

### The Eight Invariants

The kernel must enforce these eight invariants with no exceptions:

1. Every node has exactly one owner, or is explicitly unowned
2. You cannot move a node while any Ref edge points to it
3. You cannot move a node while any ref_mut lease is active on it
4. Only one ref_mut lease can exist for a node at any time
5. Ownership edges cannot form cycles (verified at edge creation)
6. Dropping an owner cascades to all owned nodes deterministically
7. Weak edges never prevent deletion - `upgrade()` can return None
8. A node's data cannot be mutated without a ref_mut lease

An invalid graph state is **unreachable**, not just unlikely.

---

## Quick Examples

### Creating and Persisting a Node

```rust
// Create a node owned by the current process - ephemeral, not journaled
let buffer = liblob::create()
    .attr("type", "text")
    .attr("name", "draft.txt")
    .owner(self_process_id)
    .build()?;

// Edit the buffer...
liblob::ref_mut(buffer, |data| {
    data.extend_from_slice(b"Hello, world!");
})?;

// Save - move to unowned, node becomes persistent and journaled
liblob::move(buffer, UNOWNED)?;
// Now survives process death, session end, and reboots
```

### Querying Nodes

```rust
// Find all images created by alice
let results = liblob::query("type:image user:alice")?;

// Find everything firefox has ever created
let results = liblob::query("created_by_binary:[firefox-node-id]")?;

// Find recent documents
let results = liblob::query("type:document created_at:>2024-01-01")?;
```

### Package Installation

```rust
fn install(manifest: Manifest) -> Result<()> {
    let pkg = liblob::create()
        .attr("type", "package")
        .attr("name", manifest.name)
        .owner(self_process_id)   // owned during install - ephemeral
        .build()?;

    for file in manifest.files {
        liblob::create()
            .attr("name", file.name)
            .data(download(file.url)?)
            .owner(pkg)
            .build()?;
    }

    liblob::move(pkg, UNOWNED)?;  // commit - package is now persistent
    Ok(())
}

fn uninstall(pkg: NodeId) -> Result<()> {
    store.drop_node(pkg)  // one line - cascade handles everything
}
```

---

## Memory Model

LOB provides three tiers of memory:

**Anonymous Memory** - Traditional stack and heap (`malloc`, `mmap`) for performance-critical code and temporary data. Fast, lightweight, dies with the process.

**Ephemeral Nodes** - Nodes owned by processes or sessions. Full provenance tracking, visible in the graph, not journaled. Die when the owner exits.

**Persistent Nodes** - Nodes owned by unowned entities or unowned themselves. Journaled to disk, survive reboots, persist indefinitely.

See [node_store.md](docs/node_store.md) for details.

---

## Key Benefits

**Structural correctness** - Invalid graph states are unreachable, not just unlikely. No fsck, no corruption recovery.

**Automatic cleanup** - Cascade deletion means no cleanup code, no resource leaks, no orphaned temp files.

**Complete provenance** - Every node records which binary created it, which user, which session, and when. Unforgeable.

**Query-based discovery** - Find data by what it is, not where it is. No directory navigation required.

**Reproducible builds** - Content addressing and lockfiles make environments reproducible by construction.

**Capability-based security** - Access control is structural. A process cannot access nodes it has no edges to.

See [security.md](docs/security.md) for the full security model.

---

## User Interface

The primary interface is a flat node browser with no directory tree - only a search bar and saved views. A second tab renders the same data as a visual graph showing ownership islands, relationships, and provenance chains.

See [user_experience.md](docs/user_experience.md) for details.

---

## POSIX Compatibility

LOB ships `libposix`, a userspace library that translates POSIX calls into LOBNS operations. Legacy programs compiled against `musl` and `libposix` run without modification. The kernel does not know or care about POSIX.

See [libposix.md](docs/libposix.md) for the translation layer.

---

## Documentation

- [node_store.md](docs/node_store.md) - Node and edge definitions, syscall API, memory model, journaling
- [security.md](docs/security.md) - Access control, sandboxing, provenance, resource limits
- [user_experience.md](docs/user_experience.md) - Node browser, query language, graph visualization
- [libposix.md](docs/libposix.md) - POSIX compatibility layer
- [reproducibility.md](docs/reproducibility.md) - Content addressing, lockfiles, hermetic builds
- [implementation.md](docs/implementation.md) - Testing strategy, development phases, project structure
- [lob_overview.md](docs/lob_overview.md) - Original comprehensive document

---

## Implementation

LOB is implemented in Rust because its type system is isomorphic to the ownership semantics the kernel enforces. Using anything else would mean reimplementing a worse version of that type system before the actual work could begin.

LOB is currently in Phase 0 (proof of concept on Linux/Windows.) The LOBNS node store is implemented as a pure `no_std` library with exhaustive testing (property-based fuzzing, fault injection, 100% branch coverage).

Under active development: node browser, lob-shell

See [implementation.md](docs/implementation.md) for the full roadmap.

---

## License

See [LICENSE](LICENSE) for details.
