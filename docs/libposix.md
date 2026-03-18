# libposix - Bootstrap Compatibility Layer

libposix is a bootstrap shim, not a POSIX compatibility guarantee. Its sole purpose is to run enough legacy tooling to build the native LOB toolchain. Once that is done, libposix has served its purpose.

The goal is narrow and explicit: run `rustc`, `cargo`, `musl`, a C compiler, and minimal coreutils equivalents long enough to reach a self-hosted LOB build. libposix is a ladder you kick away once you have climbed it.

---

## Design Philosophy

libposix is a pure translation layer maintained entirely in userspace. The kernel does not know or care about POSIX. All POSIX semantics — paths, file descriptors, permissions — are implemented by mapping them onto LOBNS primitives.

libposix is not Wine. It is not WSL. It makes no attempt at general POSIX compatibility. Anything outside the bootstrap target is simply not supported, not broken — out of scope by design.

---

## Bootstrap Target

The supported programs are:

- `rustc` and `cargo`
- `musl` libc
- A C compiler (`gcc` or `clang`)
- A minimal coreutils subset — either `uutils` components or custom node-native replacements

This set has a well-understood syscall footprint. libposix implements what these programs actually need, verified by running them, not by consulting the POSIX specification.

Everything outside this set is unsupported until there is a concrete reason to add it.

---

## How Hierarchy Is Simulated

A directory tree is a specific pattern of ownership edges maintained entirely within libposix. A directory is a node with `attr("type", "directory")` owning its children. Path resolution is a query traversal — no new kernel primitives are required.

```rust
// A directory is just a node that owns other nodes
[dir: /home] ──own──► [dir: alice]
                          ├──own──► [file: notes.txt]
                          └──own──► [file: todo.txt]
```

Path resolution walks the ownership chain:

```rust
fn resolve_path(path: &str) -> Result<NodeId> {
    let components = path.split('/').filter(|s| !s.is_empty());
    let mut current = ROOT_NODE_ID;

    for component in components {
        let query = format!("owner:{} name:{}", current, component);
        current = liblob::query(&query)?
            .into_iter()
            .next()
            .ok_or(ENOENT)?;
    }

    Ok(current)
}
```

Hierarchy is a query pattern maintained by libposix. It is not a property of LOBNS itself.

---

## Implemented Syscalls

Only what the bootstrap target actually requires. This list grows only when a concrete program needs it.

| POSIX call | libposix translation |
|---|---|
| `open(path, flags)` | resolve path query → acquire Ref or RefMut handle |
| `read(fd, buf)` | read from node data at stored offset |
| `write(fd, buf)` | write via ref_mut lease |
| `close(fd)` | release Ref or RefMut lease |
| `stat(path)` | query node attributes → populate stat struct |
| `mkdir(path)` | create node with `type:directory`, owned by parent |
| `unlink(path)` | drop owned node or remove Ref edge |
| `rename(old, new)` | update `name` attribute + move ownership atomically |
| `pipe()` | create nameless node, return two handles to it |
| `access(path, mode)` | check `posix_mode` attribute against requested access |

Symlinks, hardlinks, mount points, device files, and special filesystems (`/proc`, `/sys`) are not emulated. The bootstrap target does not require them. LOBNS replaces what they were solving with better primitives — these are not gaps, they are deliberate omissions.

---

## File Descriptors

A file descriptor is a handle to a Ref or RefMut lease on a node, plus an offset for sequential reads and writes:

```rust
struct FileDescriptor {
    node:   NodeId,
    lease:  Lease,      // Ref or RefMut
    offset: u64,
    flags:  OpenFlags,  // O_RDONLY, O_WRONLY, O_RDWR, O_APPEND, etc.
}

enum Lease {
    Ref(RefHandle),
    RefMut(RefMutHandle),
}
```

The fd table is maintained entirely in userspace by libposix. The kernel knows nothing about file descriptors — it only knows about Ref and RefMut leases.

---

## Permissions

POSIX permissions are stored as attributes and checked by libposix:

```rust
posix_mode: 0644
posix_uid:  1000
posix_gid:  1000
```

libposix checks these attributes before granting access. The kernel does not enforce POSIX permissions — it enforces the ownership model. For the bootstrap target this is sufficient. Native LOB applications ignore POSIX permissions entirely and rely on the structural capability model.

---

## The Root Directory

The root `/` is a well-known unowned persistent node created at system initialisation. All POSIX path resolution is relative to it.

```rust
const ROOT_NODE_ID: NodeId = NodeId(1);

[node: 1]
type: directory
name: /
owner: None  // unowned, persists forever
```

---

## Integration with musl

libposix is linked with musl libc. musl provides the standard C library interface and calls into libposix for filesystem operations.

```
┌─────────────────┐
│  C application  │
└────────┬────────┘
         │
    ┌────▼─────┐
    │   musl   │  (libc, malloc, threads, etc.)
    └────┬─────┘
         │
    ┌────▼──────┐
    │  libposix │  (filesystem translation layer)
    └────┬──────┘
         │
    ┌────▼──────┐
    │   liblob  │  (native LOBNS syscall wrappers)
    └────┬──────┘
         │
    ┌────▼──────┐
    │   kernel  │  (LOBNS syscalls)
    └───────────┘
```

A program compiled against musl and libposix runs without modification within the bootstrap scope. It sees a traditional filesystem. Under the hood every operation is translated into LOBNS queries and ownership operations.

---

## Nodes Created by POSIX Programs

Nodes created by POSIX programs are real LOBNS nodes, queryable and browsable alongside natively created nodes:

```rust
[node: 12847]
type: file
name: output.txt
posix_mode: 0644
posix_uid: 1000
created_by_binary: [node: 9234]  // gcc
created_by_user: alice
created_at: 1704067200
content_hash: [...]
data: [...]
```

The POSIX layer is a view. The underlying data is always LOBNS nodes. A file created by `gcc` during a bootstrap build is as queryable and provenance-tracked as anything created by a native LOB application.

---

## Performance

libposix path resolution requires multiple queries to walk the ownership chain. This is acceptable for the bootstrap target. It is not the right interface for production use.

Native LOB applications bypass libposix entirely:

```rust
// libposix path: open → resolve → query chain → lease
// Native path: node ID directly → lease
let data = liblob::ref_read(node_id, |data| data.to_vec())?;
```

Once the native toolchain is built on LOB, libposix recedes. New tooling speaks nodes directly. The translation overhead and the hierarchy illusion disappear together.

---

## What Comes After libposix

The bootstrap target is a means to an end. Once rustc and cargo run on LOB natively, the interesting work begins:

- Node-native shell commands that speak the graph directly instead of emulating Unix tools
- A native git client whose object model lives in the node store rather than a `.git` directory
- Editor integrations that treat buffers as nodes with provenance rather than files with paths

libposix exists to get there. It is not the destination.

---

See also:
- [README.md](README.md) - Overview and quick start
- [node_store.md](node_store.md) - Node and edge definitions
- [user_experience.md](user_experience.md) - Native query interface
- [implementation.md](implementation.md) - Development phases and roadmap