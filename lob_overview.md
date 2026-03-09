# LOB Operating System

LOB (Lifetime, Own, Borrow) is an operating system built around the idea of replacing the traditional POSIX/NT style filesystem with a node store called **LOBNS** (LOB Node Store). Instead of files and processes, there are only nodes with relationships (edges). The node store spans memory and disk transparently — there is one store, not two. It follows Rust's ownership, lifetimes, and borrowing rules, enforced by the kernel at every syscall boundary.

---

## The Problem With Conventional Filesystems

Rust's ownership system was designed to solve a specific problem — you cannot have both aliasing (multiple references to the same thing) and mutation (changing the thing) simultaneously. Filesystems have exactly the same problem and have never formally solved it. Instead they bolt on reactive measures:

- **File locking** — advisory, often ignored
- **Permissions** — coarse, do not express aliasing
- **Journaling** — repairs corruption after the fact
- **fsck** — detects invariant violations after they have already happened

These are all reactive. They detect or repair violations. LOBNS is proactive — the invariants cannot be violated because the kernel structurally prevents it, the same way rustc prevents memory unsafety.

---

## The Eight Invariants

The kernel must enforce these eight invariants with no exceptions:

1. Every node has exactly one owner, or is explicitly unowned
2. You cannot move a node while any Ref edge points to it
3. You cannot move a node while any ref_mut lease is active on it
4. Only one ref_mut lease can exist for a node at any time
5. Ownership edges cannot form cycles (verified at edge creation)
6. Dropping an owner cascades to all owned nodes deterministically
7. Weak edges never prevent deletion — `upgrade()` can return None
8. A node's data cannot be mutated without a ref_mut lease

An invalid graph state is **unreachable**, not just unlikely. The store cannot become inconsistent the way ext4 can. The only threat is hardware failure mid-write, which the journal handles, and after journal recovery all invariants are restored.

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

## Node and Edge Definitions

Every entity in the system — running processes, installed packages, user documents, application data, configuration, hardware devices — is a node. The struct is split into two clearly separated namespaces: fields the kernel owns and enforces, and fields the application owns.

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

The separation between kernel fields and application fields is enforced at the syscall boundary. Kernel fields do not exist in the attr namespace — there is no string key that aliases to a kernel field, no way to shadow or spoof kernel-stamped data from userspace.

### Provenance Is Unforgeable

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

## Persistence — Unowned by Default

Nodes in LOBNS are **persistent by default**. An unowned node exists indefinitely. It survives reboots, process deaths, and session ends. Nothing needs to explicitly mark a node as persistent — existence is persistence.

Ephemerality is the exception, not the rule. A node is ephemeral only when it is explicitly owned by something that will eventually be dropped — a process, a session, a transaction. When the owner is dropped, the cascade reaches the owned nodes and they disappear automatically. No flags, no tiers, no kernel heuristics.

**Saving a document is just releasing ownership:**

```rust
// Buffer is owned by the editor process — ephemeral
let buffer = liblob::create()
    .attr("type", "text")
    .owner(self_process_id)
    .build()?;

// Save — move to unowned, node becomes persistent
liblob::move(buffer, UNOWNED)?;
// Now survives process death, session end, and reboots
// Found by query, not by location
```

`move(node, UNOWNED)` is the commit point. Before it, work is transactional — process death rolls it back automatically. After it, the node is independent and permanent.

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

### Ref and Weak Edges Are the Connective Tissue

On disk, Ref and Weak edges express relationships and record history. They are what connects the flat islands into a queryable semantic graph:

```
[firefox profile] ──weak──► [firefox binary]     "created by this program"
[bookmark: github] ──ref──► [firefox profile]    "belongs to this profile"
[download: cat.jpg] ──ref──► [bookmark: github]  "downloaded from this page"
[notes.txt v2] ──weak──► [notes.txt v1]          "previous revision"
```

None of these edges affect lifetime. They are the permanent record of how everything in the system relates to everything else — queryable at any time, traversable in the graph view.

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

## No Need for Symlinks, Hardlinks, or Permissions

### Symlinks and Hardlinks

Symlinks and hardlinks are solutions to problems LOBNS does not have. Hardlinks exist because inodes can only live in one directory — in LOBNS location is irrelevant, a node is found by query. Symlinks exist because hardlinks cannot cross filesystem boundaries — in LOBNS there is one store and no concept of location.

The sharp edges these primitives introduce — dangling symlinks, symlink loops, TOCTOU security races, silent survival after `rm` — are entirely absent from LOBNS. They exist in `libposix` as a thin emulation layer for legacy software and nowhere else.

### Permissions

LOBNS does not need a separate permission system because the ownership model *is* access control. A process can only mutate a node if it holds a `ref_mut` lease. It can only drop a node if it owns it. It can only read a node if it holds a Ref. The kernel enforces this at every syscall boundary as a structural consequence of the ownership graph, not as a permission check bolted on afterward.

---

## The Journal

The journal sits between the in-memory node store and raw storage. Its sole responsibility is ensuring every node write is atomic — a partial write caused by power loss leaves the system in a recoverable state.

The journal is intentionally as simple as possible: a linear append-only log with a single commit record, similar to SQLite's WAL. Every mutation is written to the journal first. A commit record is written only when the mutation is complete. On boot, the kernel replays committed but not yet checkpointed entries and discards uncommitted ones. After recovery, all eight invariants are verified.

Simplicity is a correctness strategy. The simpler the journal, the more obviously correct it is, and the easier it is to test with fault injection.

---

## Testing Strategy

LOBNS takes direct inspiration from SQLite's approach to correctness — 100% branch coverage, every possible failure point tested, a test suite orders of magnitude larger than the library itself. LOB applies the same discipline.

### Invariant Checking

In debug builds, every mutation runs a full graph-wide invariant check immediately after it completes. Violations are caught at the exact operation that caused them, not discovered later through mysterious symptoms.

### Property-Based Testing

`proptest` generates thousands of random operation sequences automatically. After every single operation in every sequence, all eight invariants are verified. `proptest` finds and shrinks the minimal failing sequence automatically.

### Fault Injection

The journal layer exposes a `StorageBackend` trait. In tests, a `FaultInjectingBackend` simulates power loss after any specific write. Every operation is tested with power loss injected at every possible write boundary, then the simulated system boots from the journal and all invariants are verified after recovery.

### Mutation Testing

`cargo-mutants` automatically modifies the source and verifies that every mutation causes a test failure. Any mutation that passes is a real gap in coverage.

### The Node Store as a Pure Library

The node store is a `no_std` Rust library that runs with `cargo test` on a development machine, independent of any hardware or kernel. The node store is proven correct in safe Rust before it ever runs on bare metal.

---

## The Node Browser

The primary user interface for LOB is a flat node browser with no directory tree to navigate — only a search bar and a sidebar of saved views.

### The Search Bar

```
notes                            fuzzy name and content match
type:image                       exact attribute match
type:image user:alice            multiple attributes, implicitly ANDed
"meeting notes"                  phrase match
created_at:>2024-01-01           kernel field with comparison
created_by_binary:[node-id]      everything a specific program ever created
created_by_session:abc123        everything from a specific session
/^report_\d{4}/                  regex against name attribute
type:image,video                 OR within a single attribute
!type:view                       negation
```

The query language addresses both kernel fields and application attrs with identical syntax. The query engine knows which terms map to kernel struct fields and which map to the application attr map — the user never needs to care about this distinction.

### Views

A view is a node in LOBNS with `type:view` and a serialized query as its data. Views are first-class — owned, shared, queried like anything else. A process can hold a Ref to a view node and receive push notifications when the result set changes. The browser updates in real time with no polling.

### The Graph View

A second tab renders the same data as a visual graph — nodes as circles, edges as lines, ownership islands as natural clusters. Ownership edges are solid lines. Ref edges are dashed. Weak provenance edges are dotted. Node type determines colour.

The graph view makes immediately visible things no conventional OS can show: the blast radius of dropping a node, which nodes survive a package removal, the full provenance chain from any node back to the binary that created it, and anomalous behaviour — a process holding edges to nodes it has no legitimate reason to access is structurally visible.

---

## Programs as Islands of Nodes

When a package manager installs a program it creates a small owned island — a package node that owns binaries and resources, with Ref edges to shared libraries. The package manager does not own what it installs in any long-term sense. It is a tool that runs, creates nodes, and exits. The relationship is recorded as a weak backlink — provenance, not ownership.

While installation is in progress, the partially-built package is owned by the package manager process. If the install is interrupted, the cascade automatically removes the partial installation. When installation succeeds, the package is moved to unowned and becomes persistent:

```rust
fn install(manifest: Manifest) -> Result<()> {
    let pkg = liblob::create()
        .attr("type", "package")
        .attr("name", manifest.name)
        .owner(self_process_id)   // owned during install — ephemeral
        .build()?;

    for file in manifest.files {
        liblob::create()
            .attr("name", file.name)
            .data(download(file.url)?)
            .owner(pkg)
            .build()?;
    }

    liblob::move(pkg, UNOWNED)?;  // commit — package is now persistent
    Ok(())
    // If we never reach move(), process death cascades everything away
}

fn uninstall(pkg: NodeId) -> Result<()> {
    store.drop_node(pkg)  // one line — cascade handles binaries and resources
}
```

Application data created by the program — profiles, preferences, saved documents — is not owned by the package and survives uninstallation. It has `app:firefox` as an attribute and a weak backlink to the binary. The user is shown these nodes and decides what to keep.

---

## Nix-Style Reproducibility

LOBNS makes reproducible builds and environments a natural consequence of the data model.

### Content Addressing

Every node carries a `content_hash` — a BLAKE3 hash of its data, attributes, and the content hashes of all owned children recursively, like Git trees. Two nodes with identical content and identical dependency closures have identical content hashes. Any node can be verified against its expected hash at any time.

### Lockfiles

A lockfile is the content hashes of an environment node's entire reachable closure. Given a lockfile and a package repository, any machine can reconstruct the exact environment. The lockfile is trivially derived from the graph and always complete by construction.

### Hermetic Builds

When a build process starts, the kernel gives it Ref edges to declared dependencies and Own edges to output nodes. It cannot access anything else — those nodes are not reachable from its process node. Build hermeticity is a structural property of the access model.

### Garbage Collection

An unowned node with refcount zero is unreachable by definition and can be collected immediately. There is no separate GC pass and no store to scan — unreachability is already encoded in the graph.

---

## Security

LOB's security model is built on structural access control — capabilities enforced by the kernel as a consequence of the ownership graph, not as permission checks bolted on afterward. The model extends naturally to multi-user systems, sandboxing, resource limits, and cryptographic provenance.

### Structural Access Control

**Access control is structural.** A process cannot access a node it has no edge to. Not "should not" — cannot. The kernel will not return a node not reachable from the requesting process.

**Mutation requires exclusive leases.** Two processes cannot hold `ref_mut` on the same node simultaneously. Data races on persistent storage are structurally impossible.

**Provenance is unforgeable.** Every node carries kernel-stamped `created_by_process`, `created_by_binary`, and `created_by_user` fields no application can modify or spoof. Anomalous behaviour is permanently visible in the graph.

**No TOCTOU races.** A process resolves a node ID once and holds a Ref. The node cannot be replaced under the process while the Ref is held.

**Complete system observability.** Every node records which binary created it, which user created it, which session was active, and when. The full causal history of the system is the weak edge graph — not a log file, not a separate audit subsystem. It is simply the shape of the data.

### Multi-User Identity

User identity is a kernel concept, not an application convention. When a session is created, the kernel assigns it a UserId. Every process spawned in that session inherits that UserId. Every node created by that process is stamped with `created_by_user`, which is unforgeable.

The `user:alice` attribute that may appear in application attrs is a display hint for the browser, not an identity claim. Actual access decisions are made on `created_by_user`, which is kernel-stamped and cannot be spoofed.

Privilege escalation is prevented through capability nodes rather than setuid bits. A privileged binary owns a capability node that grants it elevated permissions. When another process runs that binary, the kernel checks for the capability node and grants the session the relevant permissions. Only a process with the appropriate capability can create a session with an elevated UserId.

### Query Scoping and Capability Distribution

Queries are scoped to the reachable set of the querying process. A query only returns nodes reachable from the process through its existing edges. A sandboxed process cannot discover nodes it has no edges to, even if those nodes match its query predicate. This prevents isolated processes from communicating by creating nodes with specific attributes — neither can see the other's nodes.

Capabilities are distributed through three mechanisms:

**Init-granted initial capabilities** — when the init process spawns a new process, it grants that process Ref edges to the nodes it needs. A browser process is created with Refs to the network stack node, the display node, and the user's bookmark nodes. It starts with exactly those capabilities and discovers nothing else.

**Explicit peer grants** — a process with a node can send its ID to another process via IPC. The receiving process calls `ref(id)` and the kernel checks that the granting process actually has access before allowing the ref. This is the standard capability passing mechanism.

**Scoped discovery within reachable set** — a process can query for nodes within its existing reachable set. This is not a new capability grant — it is a convenient way to traverse the graph of nodes the process already has access to.

The combination of these three mechanisms is a complete capability distribution story. Every process starts with minimal authority granted by init, receives additional capabilities explicitly from peers, and discovers related nodes within its existing access scope.

### Weak Edge Restrictions

A process can only create a weak edge to a node it already has a Ref to. You cannot create a weak edge to an arbitrary node just because you know its ID. This prevents weak edges from being used as a side channel to observe nodes you have no legitimate access to.

For nodes you do have access to, the weak edge adds no new information — you can already observe the node directly through your Ref. The only remaining channel is that two processes sharing a Ref to the same node can observe each other's access patterns through timing. This is a narrow side channel and requires shared nodes, which can be avoided for sensitive data through design.

### Resource Limits and Quotas

Resource limits are enforced through quota nodes owned by the session. A quota node specifies maximum resource usage and the kernel maintains current usage counters:

```rust
[node: quota]
type: quota
owner: [session-node]
max_nodes: 10000
max_storage: 10_GB
max_refs: 50000
max_ownership_depth: 100      // bound cascade deletion cost
current_nodes: 4821           // kernel maintains atomically
current_storage: 2.1_GB
current_refs: 1203
```

The kernel checks the quota on every `create()` and `ref()` call. A process cannot create a node if doing so would exceed its session quota. Cascade deletion decrements the counters automatically.

The `max_ownership_depth` bound is essential — without it, a malicious process could create an ownership chain millions of nodes deep and make cascade deletion unbounded. With it, the kernel can guarantee O(max_depth × max_nodes) worst case for any drop operation. A system-wide hard ceiling on `max_ownership_depth` ensures no session can create pathologically expensive cascade deletions.

### Sandboxing

Sandboxing is a natural consequence of the capability model. A sandboxed process is created with minimal initial authority and cannot escape without cooperation from an unsandboxed process:

```rust
fn create_sandbox(initial_refs: &[NodeId], quota: Quota) -> Result<NodeId> {
    let sandbox_session = kernel::create_session(
        UserId::Untrusted,
        quota,
        QueryScope::ReachableOnly,
    )?;
    
    for node_id in initial_refs {
        liblob::make_ref(*node_id, sandbox_session)?;
    }
    
    Ok(sandbox_session)
}
```

The sandboxed process starts with exactly the refs it was given and can see nothing else. It cannot exhaust resources beyond its quota. It cannot query for nodes it has no edges to. Escaping the sandbox requires obtaining a Ref to something outside it, which requires a process outside the sandbox to grant one.

Sandbox return values pass through a one-way membrane — data can flow out but capabilities cannot. Any NodeIds in return values are filtered out. The sandbox can compute and return results, but it cannot smuggle out Refs to nodes it created:

```rust
fn sandbox_exec(sandbox: NodeId, binary: NodeId) -> Result<Vec<u8>> {
    let result = exec_in_context(sandbox, binary)?;
    
    // Result can contain data but not NodeIds
    // Any NodeIds in result are filtered out
    Ok(result.data_only())
}
```

This is the membrane pattern from capability OS research — a boundary that allows data flow but prevents capability leakage.

### Capability Attenuation

The Ref model is all-or-nothing — if you have a Ref to a node, you can read all of it. Attenuation is achieved through proxy nodes that wrap another node and restrict what can be seen or done through it:

```rust
// Create an attenuated view of a node
let limited = liblob::create()
    .attr("type", "attenuation")
    .attr("target", target_node_id)
    .attr("allowed_attrs", "name,type,created_at")
    .attr("expires_at", now + 3600)
    .attr("max_reads", 100)
    .owner(self_process_id)
    .build()?;

// Hand the attenuated node to the other process
liblob::move(limited, other_process)?;
```

The kernel enforces the attenuation when the holder tries to read through the proxy. The target node itself is never handed over — only the attenuated wrapper is. Revoking access is just dropping the wrapper node.

This is consistent with the "everything is a node" model. Attenuation is not a new kernel primitive — it is a specific use of the existing node and edge model.

### Integrity Levels

Integrity is not a stored field — it is a property computed by walking the provenance chain. A node created by a signed binary is high-integrity. A node created by an unsigned binary or a sandboxed process is low-integrity:

```rust
fn integrity_level(node: NodeId) -> IntegrityLevel {
    match node.created_by_binary {
        None => IntegrityLevel::Kernel,  // kernel-created
        Some(binary) => {
            if binary.signature.is_some() && verify_signature(binary) {
                IntegrityLevel::Signed  // trusted publisher
            } else {
                IntegrityLevel::Unsigned  // user-compiled or untrusted
            }
        }
    }
}
```

The kernel provides this as a helper function. The information is already present in the graph — integrity is derived from `created_by_binary` and the signature field, not stored separately.

A system service can refuse to act on nodes below a certain integrity level. A browser's downloaded data is `Untrusted`. A config parser can check integrity before processing. Data from untrusted sources is permanently marked as such through its provenance chain, unforgeably.

### Cryptographic Provenance

Package nodes carry Ed25519 signatures over their `content_hash`. Because `content_hash` includes owned children recursively, signing a package node signs the entire dependency closure transitively:

```rust
pub signature:   Option<[u8; 64]>,  // Ed25519 signature over content_hash
pub signing_key: Option<[u8; 32]>,  // public key of signer
```

The kernel does not verify signatures by default — verification is a userspace concern done by the package manager and the shell. But the signature is a kernel-stored field, unforgeable in the same way as other kernel fields. A process cannot retroactively sign a node it didn't sign at creation.

The chain of trust: the LOB package repository signs packages with a known public key. The package manager verifies signatures before creating package nodes. The signature field is stored permanently. Any process can later verify that a binary's signature matches the repository's public key. If `created_by_binary` points at a node with a valid signature from a trusted key, you have cryptographic proof of provenance.

### Known Limitations

**Timing channels** — two processes sharing a Ref to the same node can observe each other's access patterns through timing. This is a narrow side channel and requires shared nodes. For sensitive data this can be avoided through design — do not give two processes Refs to the same sensitive node.

**Covert channels through resource exhaustion** — a process can signal to another by exhausting specific resources and observing the other's failure patterns. This is a fundamental limitation of any system with shared resources and cannot be eliminated without hardware support.

**No full information flow control** — the system does not prevent a compromised browser from exfiltrating bookmark data it has legitimate access to. Full mandatory access control (Biba, Bell-LaPadula) is not implemented. The integrity level mechanism provides the most important property — data from untrusted sources is permanently marked — but does not enforce information flow restrictions. This is a future extension.

---

## The POSIX Compatibility Layer — libposix

LOB ships `libposix`, a userspace library that translates POSIX calls into LOBNS operations. Legacy programs compiled against `musl` and `libposix` run without modification. The kernel does not know or care about POSIX.

### How Hierarchy Is Simulated

A traditional directory tree is a specific pattern of ownership edges maintained entirely within libposix. A directory is a node with `attr("type", "directory")` owning its children. Path resolution is a query traversal — no new kernel primitives required. Hierarchy is a query pattern maintained by libposix, not a property of LOBNS itself.

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

Symlinks and hardlinks are emulated entirely within libposix with no kernel support. Nodes created by POSIX programs are real LOBNS nodes with real attributes, indexed and queryable in the node browser alongside natively created nodes.

---

## Implementation Order

LOB is built in phases, with each phase being a complete usable system before the next begins.

**Phase 0 — Proof of Concept on Linux/Windows**
The LOBNS node store as a pure `no_std` Rust library with `cargo test`. A node browser as an egui application. A CLI tool and disk writer targeting a real disk image or partition, with FUSE mount on Linux. All eight invariants implemented and tested. Property-based fuzzing. Fault injection on the journal. 100% branch coverage on the node store. This phase proves the model is correct and the API is ergonomic before any hardware is involved.

**Phase 1 — Bare Metal Kernel**
Boot on target ARM hardware. UART output as the first milestone. Memory allocator, interrupt handling, timer. The node store running in RAM only. A basic syscall layer wrapping LOBNS operations.

**Phase 2 — Interactive System**
Scheduler and context switching. The LOB shell querying the in-RAM node store. liblob userspace API. A demonstrable system.

**Phase 3 — Persistence**
The journal layer, crash-consistent writes to storage. On-disk node serialization. Boot from a persisted node store. Crash recovery tested exhaustively with fault injection.

**Phase 4 — POSIX Compatibility**
libposix, musl libc integration, ELF loader, dynamic linker. Enough syscall coverage to run simple C programs, progressing toward complex software.

**Phase 5 — Native Userspace**
The node browser running natively on LOB. Native applications. Package manager. A complete self-hosting system.

---

## Project Structure

LOB follows a BSD-style monorepo — the entire system in one repository, one build system, one release. LOBNS is not a swappable component as filesystems are in Linux. It is the identity of the OS.

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