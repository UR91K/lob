# LOB Operating System

LOB (Lifetime, Own, Borrow) is an operating system built around the idea of replacing the traditional POSIX/NT style filesystem with a node store called **LOBNS** (LOB Node Store). Instead of files and processes, there are only nodes with relationships (edges). The node store operates both in memory and on disk, and follows Rust's ownership, lifetimes, and borrowing rules.

---

## The Problem With Conventional Filesystems

Rust's ownership system was designed to solve a specific problem; you cannot have both aliasing (multiple references to the same thing) and mutation (changing the thing) simultaneously. Filesystems have exactly the same problem and have never formally solved it. Instead they bolt on reactive measures:

- File locking -  advisory, often ignored
- Permissions - coarse, do not express aliasing
- Journaling - repairs corruption after the fact
- fsck - detects invariant violations after they have already happened

These are all reactive. They detect or repair violations. LOBNS is proactive - the invariants cannot be violated because the kernel structurally prevents it, the same way rustc prevents memory unsafety.

---

## The Eight Invariants

The kernel must enforce these eight invariants with no exceptions:

1. Every node has exactly one owner, or is explicitly unowned (kernel-owned)
2. You cannot move a node while any Ref edge points to it
3. You cannot move a node while any ref_mut lease is active on it
4. Only one ref_mut lease can exist for a node at any time
5. Ownership edges cannot form cycles (verified at edge creation)
6. Dropping an owner cascades to all owned nodes deterministically
7. Weak edges never prevent deletion — `upgrade()` can return None
8. A node's data cannot be mutated without a ref_mut lease

An invalid graph state is **unreachable**, not just unlikely. The filesystem cannot become inconsistent the way ext4 can. The only threat is hardware failure mid-write, which the journal handles, and after journal recovery all invariants are restored.

---

## The LOBNS Syscall API

The LOBNS syscalls use Rust semantics directly. This is not merely inspired by Rust — it is the actual ownership model, implemented as kernel syscalls operating on persistent graph nodes instead of memory.

```rust
move(node, to)      // transfer ownership, exactly like Rust's move
clone(node, to)     // duplicate a node, new ID, new owner
ref(node)           // create a Ref edge, borrow
ref_mut(node)       // create exclusive write lease, &mut
drop(node)          // explicitly release ownership, triggers cascade
weak(node)          // create a Weak edge, no lifetime implication
upgrade(weak_ref)   // attempt to promote Weak to Ref, fails if node is tombstone
```

### The Three Edge Types

| Edge | Semantics | Lifetime effect |
|---|---|---|
| `Own` | One per node maximum, target dies when dropped | Determines lifetime |
| `Ref` | Many allowed, shared borrow, read semantics | Keeps target alive |
| `Weak` | Provenance and backlinks, no lifetime effect | None — target may become tombstone |

Because ownership edges must form a DAG (verified at creation time), ownership cycles are structurally impossible. Ref cycles can exist but do not cause leaks because only Own edges determine node lifetime. This is the same reason `Weak<T>` breaks cycles in Rust.

---

## Node Structure

Every entity in the system — files, processes, devices, package metadata, configuration — is a node:

```rust
node {
    id:           u64,              // unique, never reused, never zero
    owner:        Option<NodeId>,   // None means kernel-owned
    refcount:     u32,              // number of active Ref edges pointing here
    ref_mut:      bool,             // is an exclusive lease active?
    creator:      Option<NodeId>,   // weak backlink — which process created this
    binary:       Option<NodeId>,   // weak backlink — for process nodes
    attrs:        [(key, value)],   // arbitrary queryable metadata
    edges:        [Edge],           // outgoing typed edges
    content_hash: Hash,             // BLAKE3 of canonical serialization
    created_at:   Timestamp,
    modified_at:  Timestamp,
    accessed_at:  Timestamp,
    data:         Option<[u8]>,     // raw payload
}
```

The `creator` and `binary` backlinks are **weak edges** — they record provenance without affecting node lifetime. This means you can always ask: which process created this node, which binary was that process running, and who installed that binary. The full provenance chain is in the graph permanently, queryable at any time.

---

## Names Are Artificial

In a conventional filesystem, a filename is fundamental — it is how the filesystem locates a file. The path *is* the identity. In LOBNS, identity is the node ID. A name is just an attribute, no different from `type` or `created_at`. There is nothing structurally special about it.

This means nameless nodes are not a trick or a special case — they are the natural default. A process that needs scratch space creates a nameless node it owns. When the process drops it, or the process itself dies, the node disappears via cascade deletion. No name to invent, no `/tmp` collision, no cleanup code required.

It also means the vocabulary deliberately separates itself from POSIX conventions. There are no files, no directories, no paths, no mounts. There are only nodes, edges, and queries. This is reflected in the naming:

| POSIX | LOB |
|---|---|
| file | node |
| directory | owned subgraph or view/saved query |
| path | node ID or query |
| file descriptor | ref handle |
| inode | node |
| symlink | — not needed |
| hardlink | — not needed |
| filesystem | LOBNS |

---

## No Need for Symlinks, Hardlinks, or Permissions

### Symlinks and Hardlinks

Symlinks and hardlinks are solutions to problems LOBNS does not have.

**Hardlinks** exist because Unix inodes can only live in one directory, but the same file sometimes needs to be accessible from multiple places. In LOBNS, location is irrelevant — a node has an ID and is found by query. Multiple things can hold Ref edges to the same node. The problem hardlinks solve does not exist.

**Symlinks** exist because hardlinks cannot cross filesystem boundaries and cannot point to directories safely. In LOBNS, there is one node store and no concept of location, so the problem symlinks solve does not exist either.

The sharp edges these primitives introduce — dangling symlinks, symlink loops, silent survival of hardlinked files after `rm`, TOCTOU security races on symlink resolution — are entirely absent from LOBNS. They exist in `libposix` as a thin emulation layer for legacy software and nowhere else.

### Permissions

Traditional Unix permissions are coarse and location-based — a directory's permission bits gate access to everything inside it. LOBNS does not need a separate permission system because the ownership model *is* the access control system. A process can only mutate a node if it holds a `ref_mut` lease. It can only drop a node if it owns it. It can only read a node if it holds a `Ref`. The kernel enforces this at every syscall boundary — not as a permission check bolted on afterward, but as a structural consequence of the ownership graph.

---

## The Journal

The journal is the layer between the in-memory node store and raw flash or disk storage. Its sole responsibility is ensuring that every node write is atomic from the reader's perspective — a partial write caused by power loss leaves the system in a recoverable state.

The journal is intentionally as simple as possible: a linear append-only log with a single commit record, similar to SQLite's WAL. Every mutation is written to the journal first. A commit record is written only when the mutation is complete. On boot, the kernel replays any committed but not yet checkpointed entries and discards any uncommitted entries. After recovery, all eight invariants are verified to hold.

Simplicity here is a correctness strategy. The simpler the journal, the more obviously correct it is, and the easier it is to test exhaustively with fault injection.

---

## Testing Strategy

LOBNS takes direct inspiration from SQLite's approach to correctness. SQLite achieves 100% branch coverage, tests every possible allocation and I/O failure point, and has a test suite orders of magnitude larger than the library itself. LOB applies the same discipline to the node store.

### Invariant Checking

In debug builds, every mutation runs a full graph-wide invariant check after it completes. If any of the eight invariants are violated, the exact operation that caused the violation is identified immediately. Invalid states are caught at the operation that causes them, not three operations later when something mysteriously fails.

### Property-Based Testing

Rather than writing individual test cases, the test suite describes properties that must hold for all possible inputs and uses `proptest` to generate thousands of random operation sequences automatically. After every single operation in every sequence, all eight invariants are verified. `proptest` finds the minimal operation sequence that breaks any invariant — if a bug only manifests after 47 specific operations, it gets shrunk to the minimal 3-operation reproduction case.

### Fault Injection

The journal layer exposes a `StorageBackend` trait. In tests, a `FaultInjectingBackend` replaces the real storage. It can be configured to simulate power loss after any specific write. The test suite runs every complex operation with power loss injected at every possible write boundary, then boots the simulated system from the journal and verifies that all invariants hold after recovery. Every possible crash scenario for every operation is tested exhaustively before the code runs on real hardware.

### Mutation Testing

After the test suite is comprehensive, `cargo-mutants` automatically modifies the source — flipping comparisons, removing checks, changing error types — and verifies that every mutation causes a test failure. Any mutation that does not fail a test is a real gap in coverage.

### The Node Store as a Pure Library

The node store is a `no_std` Rust library that runs on the development machine with `cargo test`, independent of any hardware or kernel. All of the above testing runs on a laptop. The node store is proven correct in safe Rust before it is ever compiled for bare metal. This is the foundational investment that makes everything above it trustworthy.

---

## The Node Browser

The primary user interface for LOB is a flat node browser. There are no directories to navigate, no tree to click through. There are only two elements: a search bar and a sidebar of saved views.

### The Search Bar

The search bar accepts a spectrum of input from casual to precise:

```
notes                           fuzzy name and content match
type:image                      exact attribute match
type:image location:cape-town   multiple attributes, implicitly ANDed
"meeting notes"                 phrase match
type:text created:>2024-01-01   attribute with comparison operator
creator:firefox                 everything a specific process created
/^report_\d{4}/                 regex against name attribute
type:image,video                OR within a single attribute
!type:view                      negation
```

Typing plain text does a fuzzy search over names and content. Attribute syntax unlocks precise queries. The bar is always the same bar — there is no mode switch between casual and advanced use.

### Views

The sidebar contains saved views. A view is itself a node in LOBNS with `type:view` and a serialized query as its data. Views are first-class — they can be owned, shared, moved between processes, and queried like anything else. Creating a view is as simple as running a search and saving it. Reordering the sidebar updates a `sidebar_order` attribute on the view node. Deleting a view is `drop(view_node_id)`.

Because views are nodes, they compose naturally. A view can query for other views. A view can be scoped to nodes created by a specific process or binary. A process can hold a Ref to a view node and receive push notifications when the query result set changes — the file browser updates in real time as new matching nodes are created, with no polling.

### The Graph View

A second tab renders the same data as a visual graph — nodes as circles, edges as lines, ownership subtrees as natural clusters. This view is modelled on Obsidian's graph view and VirusTotal's relationship graph.

Ownership edges are solid lines. Ref edges are dashed. Weak provenance edges are dotted. Node type determines colour. An installed program appears as an ownership island — a package node at the centre, owning its binaries, configuration, and resources, with dashed Ref edges out to shared libraries. The structure of the graph reflects the actual runtime relationships, not a manifest that might be out of date.

This view makes several things immediately visible that no conventional OS can show:

- The blast radius of dropping a node — everything in the ownership subtree is highlighted
- Which user files survive a program's removal — they are not in the ownership subtree
- The full provenance chain from any node back through its creator process to the binary that was running and the package manager invocation that installed it
- Anomalous behaviour — a process that creates nodes or takes Ref edges it has no legitimate reason to touch is structurally visible as unexpected edges

---

## Programs as Islands of Nodes

When a package manager installs a program, it creates an ownership island — a package node that owns the binary, configuration, and resources, with Ref edges to shared libraries it depends on. When the program runs, a process node extends that island, owning ephemeral scratch nodes and holding Ref edges to any user data it has open.

The provenance chain is permanent and queryable. Every node records its `creator` as a weak backlink to the process that created it, and every process records which binary it is running. The query `creator:firefox type:image` returns every image node ever created by any instance of Firefox. No log files, no external audit tooling — the answer is in the graph.

---

## Installing and Uninstalling Programs

Because programs are ownership islands, installation and uninstallation are trivially simple:

```rust
// Install — create the package node island
fn install(package: PackageManifest) -> Result<NodeId> {
    let pkg = store.create()
        .attr("type", "package")
        .attr("name", package.name)
        .attr("version", package.version)
        .owner(package_manager_id)
        .build()?;

    for file in package.files {
        store.create()
            .attr("type", file.kind)
            .attr("name", file.name)
            .data(file.bytes)
            .owner(pkg)
            .build()?;
    }

    for dep in package.dependencies {
        let dep_node = resolve_by_hash(dep.content_hash)?;
        store.make_ref(dep_node)?;
    }

    Ok(pkg)
}

// Uninstall
fn uninstall(pkg: NodeId) -> Result<()> {
    store.drop_node(pkg) // cascade deletion handles everything
}
```

Uninstall is one line. The cascade deletion removes every node owned by the package node. Shared libraries are not removed because they are Ref edges, not Own edges — they disappear only when nothing references them anymore. The package manager does not maintain a separate database of installed files. The ownership graph is that database.

---

## Nix-Style Reproducibility

LOBNS makes reproducible builds and environments a natural consequence of the data model rather than an elaborate workaround.

### Content Addressing

Every node carries a `content_hash` — a BLAKE3 hash of its data, attributes, and the content hashes of all its owned children recursively, like Git trees. Two nodes with identical content and identical dependency closures have identical content hashes. A specific version of a package can be requested by hash. Any node can be verified against its expected hash at any time.

### Lockfiles

A lockfile is just the content hashes of an environment node's entire reachable closure. Given a lockfile and access to a package repository, any machine can reconstruct the exact environment. The lockfile is trivially derived from the graph and is always complete by construction.

### Hermetic Builds

When a build process starts, the kernel gives it Ref edges to its declared dependencies and Own edges to its output nodes. It has no access to anything else — not because it is told not to access other nodes, but because those nodes are not reachable from its process node. Build hermeticity is a structural property of the access model, not a sandbox bolted on afterward.

### Garbage Collection

An unreferenced package node has refcount zero and no Own edges pointing to it. It is collected automatically by the reference counting that is already built into the ownership model. There is no separate GC pass, no store to scan, no equivalent of `nix-collect-garbage`. Unreachability is already encoded in the graph.

---

## Security

LOBNS's ownership model provides security properties that conventional permission systems cannot:

**Access control is structural.** A process cannot access a node it has no edge to. Not "should not" — cannot. The kernel will not return a node that is not reachable from the requesting process's node. There is no ambient namespace to scan.

**Mutation requires exclusive leases.** Two processes cannot hold `ref_mut` on the same node simultaneously. The kernel rejects the second request. Data races on persistent storage are structurally impossible.

**Provenance is permanent.** Every node records which process created it. Anomalous behaviour — a program creating nodes or borrowing data it has no legitimate reason to touch — is visible in the graph and queryable.

**No TOCTOU races.** Symlink-based TOCTOU attacks, where privileged code checks a path and an attacker swaps a symlink before the code acts on it, do not exist. A process resolves a node ID once and holds a Ref. The node cannot be replaced under the process while the Ref is held.

---

## The POSIX Compatibility Layer — libposix

LOB ships `libposix`, a userspace library that translates POSIX calls into LOBNS operations. Legacy programs compiled against `musl` and `libposix` run without modification. The kernel does not know or care about POSIX — all translation happens in userspace.

### How Hierarchy Is Simulated

A traditional directory tree is just a specific pattern of ownership edges. A directory is a node with `attr("type", "directory")` owning its children. Path resolution is a query traversal:

```rust
fn resolve_path(path: &str) -> Option<NodeId> {
    let parts = path.split('/').filter(|s| !s.is_empty());
    let mut current = store.query()
        .attr("type", "directory")
        .attr("name", "/")
        .first()?;

    for part in parts {
        current = store.query()
            .attr("name", part)
            .owned_by(current)
            .first()?;
    }
    Some(current)
}
```

This is the entire path resolver. No new kernel primitives are required. Hierarchy is a query pattern over the ownership graph, not a structural property of the storage model.

### What libposix Emulates

| POSIX call | libposix translation |
|---|---|
| `open(path, flags)` | resolve path query → acquire Ref or RefMut handle |
| `read(fd, buf)` | read from node data at stored offset |
| `write(fd, buf)` | write via ref_mut lease |
| `stat(path)` | query node attributes → populate stat struct |
| `mkdir(path)` | create node with `type:directory`, owned by parent |
| `unlink(path)` | drop node or remove Ref edge |
| `rename(old, new)` | update `name` attribute + move ownership atomically |
| `pipe()` | create nameless node, return two handles to it |
| `symlink(target, path)` | create node with `posix_type:symlink`, target as string attribute |
| `link(old, new)` | create directory entry node with Ref to target |

Symlinks and hardlinks are emulated entirely within libposix with no kernel support. They exist for legacy compatibility and nowhere else.

### Files Created by POSIX Programs Are Still Queryable

When libposix creates a file via `open(path, O_CREAT)`, it calls `liblob::create()` with attributes inferred from the filename and content. A file named `photo.jpg` gets `attr("type", "image")`. These are real LOBNS attributes on real nodes, indexed and queryable like anything else. The node browser shows files created by legacy programs alongside natively created nodes. A background enrichment daemon can add richer attributes — EXIF data from images, document metadata from PDFs — by reading node content and writing additional attributes after creation.

---

## Implementation Order

LOB is built in phases, with each phase being a complete usable system at its level before the next begins.

**Phase 1 — Node Store Library**
The LOBNS node store as a pure `no_std` Rust library, running on the development machine with `cargo test`. All eight invariants implemented and verified. Property-based fuzzing with `proptest`. Fault injection testing of the journal layer. 100% branch coverage. Nothing moves forward until this is provably correct.

**Phase 2 — Bare Metal Kernel**
Boot on target ARM hardware (Raspberry Pi Zero 2W or similar). UART output as the first milestone — a visible character on screen proves the boot chain works. Memory allocator, interrupt handling, timer. The node store ported to bare metal, running in RAM only, no persistence yet. A basic syscall layer wrapping LOBNS operations.

**Phase 3 — Interactive System**
Scheduler and context switching. The LOB shell, querying the in-RAM node store interactively. liblob userspace API. This phase produces a system you can demo and explain — not yet persistent, but functional and illustrative of the ownership model in practice.

**Phase 4 — Persistence**
The journal layer, crash-consistent writes to flash or SD card. On-disk node serialization. Boot from a persisted node store. Crash recovery tested exhaustively with fault injection before this phase is considered complete.

**Phase 5 — POSIX Compatibility**
libposix, musl libc integration, ELF loader, dynamic linker. Enough syscall coverage to run simple C programs. The hierarchy simulation layer. Progressing toward running complex Linux software recompiled against the LOB userspace.

**Phase 6 — Native Userspace**
The node browser with list view and graph view. Native LOB applications. Package manager. A complete, self-hosting system.

---

## Project Structure

LOB follows a BSD-style monorepo — the entire system in one repository, one build system, one release. The filesystem is not a swappable component as it is in Linux. LOBNS is the identity of the OS.

```
lob/
  kernel/       # scheduler, syscalls, interrupt handling
  lobns/        # node store, ownership graph, journal
  drivers/      # hardware abstraction
  liblob/       # native userspace Rust API
  libposix/     # POSIX compatibility translation layer
  shell/        # default interactive interface
  browser/      # node browser, list and graph views
  tools/        # core system utilities
  boot/         # bootloader
  docs/         # invariant spec, syscall reference, design documents
```

Nothing in this structure uses the word "file" except `libposix`, which is exactly where the POSIX concept belongs — quarantined in the compatibility layer, not present in the native system.