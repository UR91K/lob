# libposix — POSIX Compatibility Layer

LOB ships `libposix`, a userspace library that translates POSIX calls into LOBNS operations. Legacy programs compiled against `musl` and `libposix` run without modification. The kernel does not know or care about POSIX.

---

## Design Philosophy

libposix is a pure translation layer. It maintains no kernel state. All POSIX semantics — paths, directories, file descriptors, permissions — are implemented entirely in userspace by mapping them onto LOBNS primitives.

The kernel provides nodes, edges, and queries. libposix provides the illusion of a hierarchical filesystem on top of that substrate.

---

## How Hierarchy Is Simulated

A traditional directory tree is a specific pattern of ownership edges maintained entirely within libposix. A directory is a node with `attr("type", "directory")` owning its children. Path resolution is a query traversal — no new kernel primitives required.

```rust
// A directory is just a node that owns other nodes
[dir: /home] ──own──► [dir: alice]
                          ├──own──► [file: notes.txt]
                          └──own──► [file: todo.txt]

// Each node has a "name" attribute
notes.txt: { type: "file", name: "notes.txt", posix_mode: 0644 }
alice:     { type: "directory", name: "alice", posix_mode: 0755 }
```

Path resolution walks the ownership chain:

```rust
fn resolve_path(path: &str) -> Result<NodeId> {
    let components = path.split('/').filter(|s| !s.is_empty());
    let mut current = ROOT_NODE_ID;
    
    for component in components {
        // Query for a child owned by current with matching name
        let query = format!("owner:{} name:{}", current, component);
        current = liblob::query(&query)?
            .into_iter()
            .next()
            .ok_or(ENOENT)?;
    }
    
    Ok(current)
}
```

Hierarchy is a query pattern maintained by libposix, not a property of LOBNS itself.

---

## What libposix Emulates

| POSIX call | libposix translation |
|---|---|
| `open(path, flags)` | resolve path query → acquire Ref or RefMut handle |
| `read(fd, buf)` | read from node data at stored offset |
| `write(fd, buf)` | write via ref_mut lease |
| `stat(path)` | query node attributes → populate stat struct |
| `mkdir(path)` | create node with `type:directory`, owned by parent |
| `unlink(path)` | drop node or remove Ref edge |
| `rename(old, new)` | update `name` attribute + move ownership atomically |
| `chmod(path, mode)` | update `posix_mode` attribute |
| `chown(path, uid, gid)` | update `posix_uid` and `posix_gid` attributes |
| `pipe()` | create nameless node, return two handles to it |
| `symlink(target, path)` | create node with `posix_type:symlink`, target as string attribute |
| `link(old, new)` | create directory entry node with Ref to target |
| `readlink(path)` | read `symlink_target` attribute |
| `access(path, mode)` | check `posix_mode` attribute against requested access |

---

## Symlinks and Hardlinks

Symlinks and hardlinks are solutions to problems LOBNS does not have. Hardlinks exist because inodes can only live in one directory — in LOBNS location is irrelevant, a node is found by query. Symlinks exist because hardlinks cannot cross filesystem boundaries — in LOBNS there is one store and no concept of location.

The sharp edges these primitives introduce — dangling symlinks, symlink loops, TOCTOU security races, silent survival after `rm` — are entirely absent from LOBNS. They exist in `libposix` as a thin emulation layer for legacy software and nowhere else.

### Symlink Emulation

```rust
// Create a symlink
fn symlink(target: &str, linkpath: &str) -> Result<()> {
    let parent = resolve_parent(linkpath)?;
    let name = filename(linkpath);
    
    liblob::create()
        .attr("type", "file")
        .attr("posix_type", "symlink")
        .attr("name", name)
        .attr("symlink_target", target)
        .owner(parent)
        .build()?;
    
    Ok(())
}

// Follow a symlink during path resolution
fn resolve_with_symlinks(path: &str) -> Result<NodeId> {
    let mut current = ROOT_NODE_ID;
    let mut symlink_depth = 0;
    
    for component in path.split('/').filter(|s| !s.is_empty()) {
        current = find_child(current, component)?;
        
        if get_attr(current, "posix_type")? == "symlink" {
            symlink_depth += 1;
            if symlink_depth > MAX_SYMLINK_DEPTH {
                return Err(ELOOP);
            }
            let target = get_attr(current, "symlink_target")?;
            current = resolve_with_symlinks(&target)?;
        }
    }
    
    Ok(current)
}
```

### Hardlink Emulation

```rust
// Create a hardlink
fn link(oldpath: &str, newpath: &str) -> Result<()> {
    let target = resolve_path(oldpath)?;
    let parent = resolve_parent(newpath)?;
    let name = filename(newpath);
    
    // Create a directory entry that refs the target
    liblob::create()
        .attr("type", "directory_entry")
        .attr("name", name)
        .owner(parent)
        .build()?;
    
    liblob::make_ref(target, entry)?;
    
    Ok(())
}
```

Hardlinks are directory entries that Ref the target node instead of owning it. The target survives `unlink()` of one path because it still has other Refs. This matches POSIX hardlink semantics exactly.

---

## File Descriptors

A file descriptor is a handle to a Ref or RefMut lease on a node, plus an offset for sequential reads/writes:

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

POSIX permissions (rwx, user/group/other) are stored as attributes and checked by libposix:

```rust
// Stored on each node
posix_mode: 0644
posix_uid:  1000
posix_gid:  1000
```

libposix checks these attributes before granting access. The kernel does not enforce POSIX permissions — it enforces the ownership model. libposix translates POSIX permission checks into decisions about whether to acquire a Ref or RefMut lease.

This means POSIX permissions are advisory for legacy software. Native LOB applications ignore them entirely and rely on the structural access control of the capability model.

---

## The Root Directory

The root directory `/` is a well-known unowned node with `type:directory` and `name:"/"`. It is created at system initialization and persists indefinitely. All POSIX-style paths are resolved relative to this node.

```rust
const ROOT_NODE_ID: NodeId = NodeId(1);  // well-known ID

// The root node
[node: 1]
type: directory
name: /
owner: None  // unowned, persists forever
```

---

## Integration with musl

libposix is designed to be linked with musl libc. musl provides the standard C library interface (`malloc`, `printf`, `pthread`, etc.) and calls into libposix for filesystem operations.

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

A POSIX program compiled against musl and libposix runs without modification. It sees a traditional filesystem. Under the hood, every operation is translated into LOBNS queries and ownership operations.

---

## What Is Not Emulated

libposix does not emulate:

- **Mount points** — there is one node store, no concept of mounting
- **Device files** — devices are nodes with `type:device`, accessed directly
- **Special filesystems** (`/proc`, `/sys`) — replaced by queries (`type:process`, `type:kernel_stat`)
- **Filesystem-specific features** (ext4 extended attributes, btrfs snapshots) — LOBNS has its own primitives

These are not limitations — they are features LOBNS replaces with better primitives.

---

## Performance Considerations

libposix path resolution requires multiple queries to walk the directory tree. For performance-critical code, native LOB applications should:

- Store node IDs directly instead of paths
- Use queries instead of directory traversal
- Avoid the POSIX layer entirely

libposix exists for compatibility, not performance. Native applications have direct access to the node store and can avoid the translation overhead entirely.

---

## Example: `cat` Implementation

```rust
// POSIX version using libposix
fn cat_posix(path: &str) -> Result<()> {
    let fd = open(path, O_RDONLY)?;
    let mut buf = [0u8; 4096];
    
    loop {
        let n = read(fd, &mut buf)?;
        if n == 0 { break; }
        write(STDOUT_FILENO, &buf[..n])?;
    }
    
    close(fd)?;
    Ok(())
}

// Native LOB version
fn cat_native(node: NodeId) -> Result<()> {
    let data = liblob::ref_read(node, |data| data.to_vec())?;
    io::stdout().write_all(&data)?;
    Ok(())
}
```

The native version is simpler, faster, and requires no path resolution. The POSIX version works but pays the translation cost.

---

## Nodes Created by POSIX Programs

Nodes created by POSIX programs are real LOBNS nodes with real attributes, indexed and queryable in the node browser alongside natively created nodes:

```rust
// A file created by a POSIX program
[node: 12847]
type: file
name: output.txt
posix_mode: 0644
posix_uid: 1000
posix_gid: 1000
created_by_binary: [node: 9234]  // /usr/bin/gcc
created_by_user: alice
created_at: 1704067200
content_hash: [...]
data: [...]
```

The node browser shows it. Queries find it. Native applications can ref it. The POSIX layer is just a view — the underlying data is always LOBNS nodes.

---

See also:
- [README.md](README.md) — Overview and quick start
- [node_store.md](node_store.md) — Node and edge definitions
- [user_experience.md](user_experience.md) — Native query interface
- [implementation.md](implementation.md) — Development phases
