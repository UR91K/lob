# Reproducibility

LOBNS makes reproducible builds and environments a natural consequence of the data model. Content addressing, lockfiles, and hermetic builds are not bolted-on features - they emerge directly from the ownership graph and provenance tracking.

---

## Content Addressing

Every node carries a `content_hash` - a BLAKE3 hash of its data, attributes, and the content hashes of all owned children recursively, like Git trees.

```rust
pub content_hash: [u8; 32],  // BLAKE3, updated by kernel on every write
```

Two nodes with identical content and identical dependency closures have identical content hashes. Any node can be verified against its expected hash at any time.

### Recursive Hashing

The content hash includes owned children recursively:

```rust
fn compute_content_hash(node: &Node) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    
    // Hash node data
    if let Some(data) = &node.data {
        hasher.update(data);
    }
    
    // Hash attributes (sorted for determinism)
    for (key, value) in node.attrs.iter() {
        hasher.update(key.as_bytes());
        hasher.update(&serialize(value));
    }
    
    // Hash owned children recursively
    for edge in &node.edges {
        if edge.kind == EdgeKind::Own {
            let child = store.get_node(edge.target)?;
            hasher.update(&child.content_hash);
        }
    }
    
    hasher.finalize().into()
}
```

This means signing a package node signs the entire dependency closure transitively. Verifying a package hash verifies all owned binaries and resources.

---

## Lockfiles

A lockfile is the content hashes of an environment node's entire reachable closure. Given a lockfile and a package repository, any machine can reconstruct the exact environment.


### Lockfile Format

```rust
// A lockfile is just a list of (NodeId, ContentHash) pairs
struct Lockfile {
    root: NodeId,
    nodes: Vec<(NodeId, [u8; 32])>,
}

// Generate a lockfile from an environment
fn generate_lockfile(env: NodeId) -> Result<Lockfile> {
    let mut nodes = Vec::new();
    let mut visited = HashSet::new();
    
    fn traverse(node: NodeId, nodes: &mut Vec<(NodeId, [u8; 32])>, visited: &mut HashSet<NodeId>) {
        if visited.contains(&node) { return; }
        visited.insert(node);
        
        let n = store.get_node(node)?;
        nodes.push((node, n.content_hash));
        
        for edge in &n.edges {
            if edge.kind == EdgeKind::Own || edge.kind == EdgeKind::Ref {
                traverse(edge.target, nodes, visited)?;
            }
        }
    }
    
    traverse(env, &mut nodes, &mut visited)?;
    
    Ok(Lockfile { root: env, nodes })
}
```

### Lockfile Verification

```rust
// Verify an environment matches a lockfile
fn verify_lockfile(env: NodeId, lockfile: &Lockfile) -> Result<bool> {
    for (node_id, expected_hash) in &lockfile.nodes {
        let node = store.get_node(*node_id)?;
        if node.content_hash != *expected_hash {
            return Ok(false);
        }
    }
    Ok(true)
}
```

The lockfile is trivially derived from the graph and always complete by construction. There is no separate lockfile format to maintain - it is just a serialized subset of the node store.

---

## Hermetic Builds

When a build process starts, the kernel gives it Ref edges to declared dependencies and Own edges to output nodes. It cannot access anything else - those nodes are not reachable from its process node.

```rust
fn hermetic_build(deps: &[NodeId], build_script: NodeId) -> Result<NodeId> {
    // Create a sandboxed session
    let sandbox = kernel::create_session(
        UserId::Builder,
        Quota::default(),
        QueryScope::ReachableOnly,
    )?;
    
    // Grant refs to dependencies only
    for dep in deps {
        liblob::make_ref(*dep, sandbox)?;
    }
    
    // Create output node owned by sandbox
    let output = liblob::create()
        .attr("type", "build_output")
        .owner(sandbox)
        .build()?;
    
    // Run build script
    exec_in_context(sandbox, build_script)?;
    
    // Move output to unowned
    liblob::move(output, UNOWNED)?;
    
    Ok(output)
}
```

Build hermeticity is a structural property of the access model. The build process cannot access network, cannot read user files, cannot see anything not explicitly granted. This is not enforced by a build tool - it is enforced by the kernel.

---

## Garbage Collection

An unowned node with refcount zero is unreachable by definition and can be collected immediately. There is no separate GC pass and no store to scan - unreachability is already encoded in the graph.

```rust
fn drop_node(node: NodeId) -> Result<()> {
    let n = store.get_node(node)?;
    
    // Can only drop if you own it or it's unowned with refcount zero
    if n.owner != Some(current_process_id) && !(n.owner.is_none() && n.refcount == 0) {
        return Err(EPERM);
    }
    
    // Cascade to owned children
    for edge in &n.edges {
        if edge.kind == EdgeKind::Own {
            drop_node(edge.target)?;
        }
    }
    
    // Remove from store
    store.remove(node)?;
    
    Ok(())
}
```

Garbage collection is just cascade deletion. When a node's last Ref is dropped, its refcount reaches zero. If it is unowned, it is immediately unreachable and can be collected.

### Automatic Collection

The kernel can run a background collector that finds unowned nodes with refcount zero and removes them:

```rust
fn collect_garbage() -> Result<usize> {
    let mut collected = 0;
    
    for node in store.iter() {
        if node.owner.is_none() && node.refcount == 0 {
            store.remove(node.id)?;
            collected += 1;
        }
    }
    
    Ok(collected)
}
```

This is trivial because unreachability is explicit in the graph. No mark-and-sweep, no tracing, no heuristics.

---

## Comparison with Nix

LOB shares Nix's goals but achieves them through different means:

| Feature | Nix | LOB |
|---|---|---|
| Content addressing | `/nix/store/hash-name` paths | `content_hash` field on every node |
| Lockfiles | `flake.lock` | Serialized node closure |
| Hermetic builds | Sandbox with declared inputs | Kernel-enforced reachability |
| Garbage collection | Mark-and-sweep from GC roots | Refcount zero = unreachable |
| Provenance | Store path derivations | `created_by_binary` kernel field |

Nix builds a reproducible system on top of a conventional filesystem. LOB makes reproducibility a property of the filesystem itself.

---

## Package Management

A package is an unowned node that owns binaries and resources. Installing a package is creating this node and moving it to unowned:

```rust
fn install(manifest: Manifest) -> Result<NodeId> {
    let pkg = liblob::create()
        .attr("type", "package")
        .attr("name", manifest.name)
        .attr("version", manifest.version)
        .owner(self_process_id)  // owned during install
        .build()?;
    
    for file in manifest.files {
        let node = liblob::create()
            .attr("name", file.name)
            .data(download(file.url)?)
            .owner(pkg)
            .build()?;
    }
    
    // Compute content hash of entire package
    let hash = compute_content_hash(pkg)?;
    
    // Sign if we have a signing key
    if let Some(key) = signing_key {
        let signature = sign(hash, key);
        liblob::set_signature(pkg, signature, key.public())?;
    }
    
    // Commit - package is now persistent
    liblob::move(pkg, UNOWNED)?;
    
    Ok(pkg)
}
```

The package's content hash covers all owned files. Signing the package signs the entire closure. Verifying the signature verifies everything.

### Uninstalling

```rust
fn uninstall(pkg: NodeId) -> Result<()> {
    // One line - cascade handles everything
    store.drop_node(pkg)
}
```

Application data created by the program survives because it was never owned by the package. It has `app:firefox` as an attribute and a weak backlink to the binary. The user is shown these nodes and decides what to keep.

---

## Reproducible Environments

An environment is a node that refs all packages and configuration needed for a specific task:

```rust
let env = liblob::create()
    .attr("type", "environment")
    .attr("name", "rust-dev")
    .owner(UNOWNED)
    .build()?;

// Add refs to packages
liblob::make_ref(rustc_pkg, env)?;
liblob::make_ref(cargo_pkg, env)?;
liblob::make_ref(rust_analyzer_pkg, env)?;

// Generate lockfile
let lockfile = generate_lockfile(env)?;
save_lockfile("rust-dev.lock", &lockfile)?;
```

Given the lockfile, any machine can reconstruct the exact environment by fetching packages with matching content hashes from a repository.

---

## Verification

Any node can be verified against its expected hash at any time:

```rust
fn verify_node(node: NodeId, expected_hash: [u8; 32]) -> Result<bool> {
    let n = store.get_node(node)?;
    Ok(n.content_hash == expected_hash)
}

fn verify_closure(node: NodeId, lockfile: &Lockfile) -> Result<bool> {
    for (id, hash) in &lockfile.nodes {
        if !verify_node(*id, *hash)? {
            return Ok(false);
        }
    }
    Ok(true)
}
```

Verification is cheap - just compare hashes. The kernel maintains content hashes automatically, so they are always up to date.

---

See also:
- [README.md](README.md) - Overview and quick start
- [node_store.md](node_store.md) - Content hash computation
- [security.md](security.md) - Cryptographic signatures
- [implementation.md](implementation.md) - Testing reproducibility
