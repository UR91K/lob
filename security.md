# Security Model

LOB's security model is built on structural access control — capabilities enforced by the kernel as a consequence of the ownership graph, not as permission checks bolted on afterward. The model extends naturally to multi-user systems, sandboxing, resource limits, and cryptographic provenance.

---

## Structural Access Control

**Access control is structural.** A process cannot access a node it has no edge to. Not "should not" — cannot. The kernel will not return a node not reachable from the requesting process.

**Mutation requires exclusive leases.** Two processes cannot hold `ref_mut` on the same node simultaneously. Data races on persistent storage are structurally impossible.

**Provenance is unforgeable.** Every node carries kernel-stamped `created_by_process`, `created_by_binary`, and `created_by_user` fields no application can modify or spoof. Anomalous behaviour is permanently visible in the graph.

**No TOCTOU races.** A process resolves a node ID once and holds a Ref. The node cannot be replaced under the process while the Ref is held.

**Complete system observability.** Every node records which binary created it, which user created it, which session was active, and when. The full causal history of the system is the weak edge graph — not a log file, not a separate audit subsystem. It is simply the shape of the data.

---

## Multi-User Identity

User identity is a kernel concept, not an application convention. When a session is created, the kernel assigns it a UserId. Every process spawned in that session inherits that UserId. Every node created by that process is stamped with `created_by_user`, which is unforgeable.

The `user:alice` attribute that may appear in application attrs is a display hint for the browser, not an identity claim. Actual access decisions are made on `created_by_user`, which is kernel-stamped and cannot be spoofed.

Privilege escalation is prevented through capability nodes rather than setuid bits. A privileged binary owns a capability node that grants it elevated permissions. When another process runs that binary, the kernel checks for the capability node and grants the session the relevant permissions. Only a process with the appropriate capability can create a session with an elevated UserId.

---

## Query Scoping and Capability Distribution

Queries are scoped to the reachable set of the querying process. A query only returns nodes reachable from the process through its existing edges. A sandboxed process cannot discover nodes it has no edges to, even if those nodes match its query predicate. This prevents isolated processes from communicating by creating nodes with specific attributes — neither can see the other's nodes.

Capabilities are distributed through three mechanisms:

**Init-granted initial capabilities** — when the init process spawns a new process, it grants that process Ref edges to the nodes it needs. A browser process is created with Refs to the network stack node, the display node, and the user's bookmark nodes. It starts with exactly those capabilities and discovers nothing else.

**Explicit peer grants** — a process with a node can send its ID to another process via IPC. The receiving process calls `ref(id)` and the kernel checks that the granting process actually has access before allowing the ref. This is the standard capability passing mechanism.

**Scoped discovery within reachable set** — a process can query for nodes within its existing reachable set. This is not a new capability grant — it is a convenient way to traverse the graph of nodes the process already has access to.

The combination of these three mechanisms is a complete capability distribution story. Every process starts with minimal authority granted by init, receives additional capabilities explicitly from peers, and discovers related nodes within its existing access scope.

---

## Weak Edge Restrictions

A process can only create a weak edge to a node it already has a Ref to. You cannot create a weak edge to an arbitrary node just because you know its ID. This prevents weak edges from being used as a side channel to observe nodes you have no legitimate access to.

For nodes you do have access to, the weak edge adds no new information — you can already observe the node directly through your Ref. The only remaining channel is that two processes sharing a Ref to the same node can observe each other's access patterns through timing. This is a narrow side channel and requires shared nodes, which can be avoided for sensitive data through design.

---

## Resource Limits and Quotas

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

---

## Sandboxing

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

---

## Capability Attenuation

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

---

## Integrity Levels

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

---

## Cryptographic Provenance

Package nodes carry Ed25519 signatures over their `content_hash`. Because `content_hash` includes owned children recursively, signing a package node signs the entire dependency closure transitively:

```rust
pub signature:   Option<[u8; 64]>,  // Ed25519 signature over content_hash
pub signing_key: Option<[u8; 32]>,  // public key of signer
```

The kernel does not verify signatures by default — verification is a userspace concern done by the package manager and the shell. But the signature is a kernel-stored field, unforgeable in the same way as other kernel fields. A process cannot retroactively sign a node it didn't sign at creation.

The chain of trust: the LOB package repository signs packages with a known public key. The package manager verifies signatures before creating package nodes. The signature field is stored permanently. Any process can later verify that a binary's signature matches the repository's public key. If `created_by_binary` points at a node with a valid signature from a trusted key, you have cryptographic proof of provenance.

---

## No Need for Permissions

LOBNS does not need a separate permission system because the ownership model *is* access control. A process can only mutate a node if it holds a `ref_mut` lease. It can only drop a node if it owns it. It can only read a node if it holds a Ref. The kernel enforces this at every syscall boundary as a structural consequence of the ownership graph, not as a permission check bolted on afterward.

Traditional Unix permissions (rwx, user/group/other) are not needed because:

- **Read access** — you have a Ref or you don't
- **Write access** — you have ref_mut or you don't
- **Execute access** — you have a Ref to the binary node or you don't
- **Ownership** — encoded in the ownership edge, not a separate field

The capability model subsumes traditional permissions entirely.

---

## Known Limitations

**Timing channels** — two processes sharing a Ref to the same node can observe each other's access patterns through timing. This is a narrow side channel and requires shared nodes. For sensitive data this can be avoided through design — do not give two processes Refs to the same sensitive node.

**Covert channels through resource exhaustion** — a process can signal to another by exhausting specific resources and observing the other's failure patterns. This is a fundamental limitation of any system with shared resources and cannot be eliminated without hardware support.

**No full information flow control** — the system does not prevent a compromised browser from exfiltrating bookmark data it has legitimate access to. Full mandatory access control (Biba, Bell-LaPadula) is not implemented. The integrity level mechanism provides the most important property — data from untrusted sources is permanently marked — but does not enforce information flow restrictions. This is a future extension.

---

See also:
- [README.md](README.md) — Overview and quick start
- [node_store.md](node_store.md) — Node definitions and provenance fields
- [user_experience.md](user_experience.md) — Querying by provenance
- [reproducibility.md](reproducibility.md) — Content addressing and signatures
