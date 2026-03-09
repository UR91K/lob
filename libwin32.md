# libwin32 — Win32 Compatibility Layer for LOB

libwin32 is a userspace library that translates Win32 API calls into LOBNS operations, allowing unmodified Windows applications to run on LOB. Unlike Wine, which fights against POSIX's filesystem model, libwin32 works with LOB's ownership-based node store, resulting in better performance, fewer edge cases, and more natural semantics.

---

## Design Philosophy

**Wine's Challenge**: Wine translates Win32 → POSIX → Linux kernel. The POSIX layer is a poor semantic match for Win32, requiring elaborate workarounds for paths, handles, registry, and object lifetimes.

**libwin32's Advantage**: libwin32 translates Win32 → LOBNS directly. Win32's object model (named/anonymous objects, reference-counted handles, hierarchical registry) maps naturally to LOBNS nodes, edges, and attributes.

The result is a thinner compatibility layer with less translation overhead and fewer impedance mismatches.

---

## Architecture

```
┌─────────────────────────────────────┐
│   Windows Application (PE binary)   │
│   Links against kernel32.dll, etc.  │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│          libwin32.so                │
│  (kernel32, user32, gdi32, etc.)    │
│  Rust implementation of Win32 APIs  │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│           liblob.so                 │
│   Native LOB userspace API          │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│         LOB Kernel                  │
│   LOBNS syscalls (move, ref, etc.)  │
└─────────────────────────────────────┘
```


Windows applications are compiled to PE format and link against standard Win32 DLLs (kernel32.dll, user32.dll, etc.). libwin32 provides these DLLs as native LOB shared libraries that export the Win32 API surface but implement it using LOBNS operations.

---

## Core Mappings

### Handles → Ref Edges

Win32 handles are opaque references to kernel objects. LOBNS Ref edges are opaque references to nodes. The mapping is nearly direct:

| Win32 | LOBNS |
|---|---|
| `HANDLE` | `RefHandle` (Ref edge to a node) |
| `CreateFile()` returns handle | `store.ref(node_id)` returns handle |
| `CloseHandle()` | `store.drop_ref(handle)` |
| Handle reference counting | Ref edge refcount |
| Handles survive process if inherited | Ref edges can be transferred between processes |

A Win32 HANDLE is literally a LOBNS Ref edge handle. No handle translation table needed. No locking. `CloseHandle()` maps directly to `drop_ref()`.


### Filesystem → Node Attributes

Win32 paths are translated to node queries. Unlike Wine, which walks a Unix directory tree, libwin32 queries indexed attributes:

| Win32 Path | LOBNS Query |
|---|---|
| `C:\Users\Alice\file.txt` | `attr("win32_drive", "C") && attr("win32_path", "/Users/Alice/file.txt")` |
| `\\?\C:\LongPath\...` | Same query, prefix is just parsing |
| `\\.\PhysicalDrive0` | `attr("win32_device", "PhysicalDrive0")` |

**Case Insensitivity**: LOBNS queries support a case-insensitive flag. No case-folding table. No filesystem scan. Just an indexed query with a flag.

**Drive Letters**: Drive letters are just an attribute. `C:` is the default system drive. Additional drives are mounted by creating nodes with different `win32_drive` attributes.


### Registry → Nodes

The Windows Registry is a hierarchical key-value store. LOBNS is a graph of nodes with attributes. The mapping is natural:

| Registry Concept | LOBNS |
|---|---|
| Registry key | Node with `type:registry_key` |
| Registry value | Attribute on the key node |
| Hive (HKLM, HKCU) | `win32_hive` attribute |
| Key path | `win32_path` attribute |
| Subkeys | Owned child nodes |

No separate registry file. No parsing on startup. Registry keys are just nodes, queryable like anything else.

**Registry Watching**: Win32's `RegNotifyChangeKeyValue()` maps directly to LOBNS view subscriptions. No polling. No inotify. Native push notifications from the kernel when registry keys change.


### Named Objects → Node Names

Win32 supports named kernel objects (mutexes, events, semaphores, file mappings). These are just nodes with a `win32_name` attribute. Anonymous objects (no name) are the natural default in LOBNS. Named objects are just nodes with an extra attribute. No special namespace required.


---

## File Operations

### CreateFile

Win32's `CreateFile()` is notoriously complex — it handles files, directories, devices, pipes, and more. libwin32 translates it to LOBNS operations by:
1. Resolving the path to a node ID via indexed query
2. Handling creation disposition (CREATE_NEW, OPEN_EXISTING, etc.)
3. Acquiring a Ref (read) or RefMut (write) lease on the node

**Key Differences from Wine**:
- No path translation table
- No filesystem walk
- Indexed query returns node directly
- Ref/RefMut maps naturally to read/write access
- No locking needed (kernel enforces exclusive write)


### ReadFile / WriteFile

Direct translation to LOBNS read/write operations on the node referenced by the handle. No translation layer. No buffering. Direct read/write to node data.


### FindFirstFile / FindNextFile

Win32's file enumeration maps to LOBNS queries. The directory path is resolved to a node, then a query finds all nodes owned by that directory matching the pattern (with wildcard support). Results are cached in a search handle node for subsequent `FindNextFile()` calls.

**Performance**: Directory enumeration is a single query, not a filesystem scan. Results are cached in a node. No repeated syscalls.


### ReadDirectoryChangesW

Win32's directory watching maps directly to LOBNS view subscriptions. A view is created for the directory (and optionally its subtree), and the kernel pushes notifications when nodes are created, modified, or deleted within that view.

**Key Advantage**: No inotify. No polling. The kernel pushes updates when the query result set changes. This is significantly faster than Wine's approach.


---

## Process and Thread Management

### CreateProcess

Creates a new process node owned by the parent process. The executable is resolved to a node, a process node is created with attributes for the binary and command line, and handles are inherited if requested. The PE binary is loaded and execution begins.

**Process Lifetime**: The process node is owned by its parent. When the parent drops it (or the parent dies), the process is terminated via cascade deletion. This matches Win32 semantics exactly.


### Synchronization Objects

Win32 synchronization primitives (mutexes, events, semaphores) are just nodes with specific attributes. A mutex has a `locked` attribute. An event has a `signaled` attribute. `WaitForSingleObject()` waits on attribute changes or node deletion (for process handles).

**Advantage**: Synchronization state is just node attributes. The kernel can efficiently wake waiting threads when attributes change. No separate synchronization subsystem needed.


---

## DLL Loading

### LoadLibrary

Win32 DLL loading follows a search order: application directory, system directory, Windows directory, current directory, PATH. In LOBNS, this becomes a series of queries:
1. Check nodes owned by the process's binary (application-local DLLs)
2. Check nodes with `win32_system_dll` attribute (system DLLs)
3. Resolve as full path if provided

Once found, the PE binary is loaded and a Ref edge is created from the process to the DLL node.

**Key Insight**: DLL dependencies are Ref edges. The graph naturally shows which DLLs a process has loaded. Unloading a DLL is just dropping the Ref edge.


---

## Security and Permissions

Win32 has a complex security model with ACLs, security descriptors, and tokens. libwin32 emulates this in userspace while relying on LOBNS's ownership model for actual enforcement.

### Security Descriptors

Security descriptors are stored as `win32_security_descriptor` attributes on nodes. Win32 apps can read and write them using the standard security APIs.

**Important**: These are *emulated* for Win32 compatibility. The actual access control is enforced by LOBNS's ownership model. A process cannot access a node it has no edge to, regardless of what the security descriptor says.

This means:
- Win32 apps see familiar security APIs
- But the real enforcement is structural (ownership graph)
- No way to bypass access control by manipulating security descriptors


---

## Performance Advantages Over Wine

### 1. Path Resolution
- **Wine**: Parse path → walk directory tree → check case-insensitivity → translate to Unix path
- **libwin32**: Parse path → indexed attribute query → return node ID
- **Speedup**: 5-10x for path-heavy operations

### 2. Handle Operations
- **Wine**: Lock handle table → translate HANDLE to fd → syscall → translate result
- **libwin32**: Direct LOBNS operation on Ref handle
- **Speedup**: 2-3x for handle-heavy operations

### 3. Registry Access
- **Wine**: Parse registry files on startup → cache in memory → walk tree structure
- **libwin32**: Indexed query on registry key nodes
- **Speedup**: 3-5x for registry-heavy applications (Office, Visual Studio)

### 4. File Watching
- **Wine**: Set up inotify watches → poll or block → translate events
- **libwin32**: Create view → kernel pushes updates
- **Speedup**: 5-10x for applications that watch many directories (IDEs, build tools)

### 5. DLL Loading
- **Wine**: Search multiple filesystem paths → parse PE repeatedly → manage side-by-side
- **libwin32**: Query for nodes with Ref edges → load once
- **Speedup**: 2-4x for DLL-heavy applications (games, Adobe software)

---

## Stability Advantages Over Wine

### 1. No TOCTOU Races
Wine is vulnerable to time-of-check-time-of-use races because paths can change between operations. libwin32 resolves once and holds a Ref — the node cannot change while the Ref is held.

### 2. Atomic Writes
Wine applications that crash mid-write can corrupt data files. LOBNS's journal ensures every write is atomic. Crashed apps leave the node store in a consistent state.

### 3. No Case-Sensitivity Bugs
Wine emulates case-insensitivity on case-sensitive filesystems, leading to subtle bugs. libwin32's case-insensitivity is native to the query system.

### 4. No Symlink Confusion
Wine applications can be confused by symlinks that don't exist on Windows. LOBNS has no symlinks at the kernel level — this entire class of bugs doesn't exist.

### 5. Deterministic Cleanup
Wine relies on Unix process cleanup, which can leak resources. LOBNS's ownership model guarantees cascade deletion when a process dies.

---

## Implementation Phases

### Phase 1: Core Infrastructure (Years 7-8)
- PE/COFF loader
- Basic Win32 API surface (kernel32 core functions)
- Handle management
- Path resolution and file I/O
- Process creation and management
- **Target**: Simple CLI tools work (7-Zip, wget, curl)

### Phase 2: GUI Support (Year 8-9)
- user32.dll (window management, messages)
- gdi32.dll (basic graphics)
- Registry implementation
- **Target**: Simple GUI apps work (Notepad++, WinSCP, PuTTY)

### Phase 3: Advanced Features (Year 9-10)
- Advanced file operations (memory-mapped files, async I/O)
- Synchronization primitives
- DLL search path and side-by-side assemblies
- COM basics (if needed)
- **Target**: Complex applications work (Visual Studio Code, Git for Windows)

### Phase 4: Compatibility Expansion (Year 10+)
- DirectX/OpenGL translation (or use existing projects like DXVK)
- Advanced COM
- .NET runtime support
- **Target**: Games, Office, Photoshop

---

## Advanced Win32 Features

### Alternate Data Streams (ADS)

**The node-native approach:** A file with ADS is just a node that owns child nodes, one per stream. The main stream is the node's data. Named streams are owned children with `stream_name` attributes.

```rust
[file.txt]
  data: "main content"
  
  ──own──► [stream: Zone.Identifier]
           stream_name: "Zone.Identifier"
           data: "[ZoneTransfer]\nZoneId=3"
  
  ──own──► [stream: CustomData]
           stream_name: "CustomData"
           data: "application-specific data"
```

Win32 path `file.txt:Zone.Identifier` resolves to the owned child node with `stream_name:"Zone.Identifier"`. Deleting the file cascades to all streams. Copying the file can optionally copy owned stream nodes. This is cleaner than NTFS's implementation — streams are just nodes, queryable and manageable like anything else.

**API Translation:**
- `CreateFile("file.txt:Zone.Identifier")` → query for node with `stream_name:"Zone.Identifier"` owned by the file node
- `DeleteFile("file.txt")` → drop file node, cascade deletes all streams
- `CopyFile()` with streams → clone file node and optionally clone owned stream nodes

### Reparse Points

**The node-native approach:** A reparse point is a node with `type:reparse_point` and a `reparse_tag` attribute. The reparse data is stored in the node's data field.

```rust
[junction]
  type: reparse_point
  reparse_tag: IO_REPARSE_TAG_MOUNT_POINT
  data: "C:\Target\Path"

[symlink]
  type: reparse_point
  reparse_tag: IO_REPARSE_TAG_SYMLINK
  data: "relative\path"
  flags: SYMLINK_FLAG_RELATIVE

[onedrive_placeholder]
  type: reparse_point
  reparse_tag: IO_REPARSE_TAG_CLOUD
  data: <cloud provider metadata>
```

When libwin32 encounters a reparse point during path resolution, it reads the `reparse_tag` and dispatches to the appropriate handler. Built-in tags (junctions, symlinks, mount points) are handled by libwin32. Third-party tags can be handled by userspace filter drivers that register handlers.

This is actually simpler than NTFS — reparse points are just nodes with specific attributes. No special filesystem support needed. Filter drivers are just processes that register to handle specific reparse tags.

**API Translation:**
- `DeviceIoControl(FSCTL_SET_REPARSE_POINT)` → set `type:reparse_point` and `reparse_tag` attributes
- `DeviceIoControl(FSCTL_GET_REPARSE_POINT)` → read `reparse_tag` and data attributes
- Path resolution → check for `type:reparse_point`, dispatch to handler based on `reparse_tag`

### Opportunistic Locks (Oplocks)

**The node-native approach:** Oplocks are just ref_mut leases with notification callbacks. When a process requests an oplock, it's requesting a ref_mut lease with a "notify me if someone else wants access" flag.

```rust
// Process A requests an oplock
let oplock = liblob::ref_mut(node)
    .with_oplock(OplockLevel::Exclusive)
    .build()?;

// Process B tries to access the same node
// Kernel sends notification to Process A
// Process A flushes caches and downgrades to shared oplock
// Process B's access proceeds
```

The kernel already tracks ref_mut leases and knows when conflicts occur. Oplocks are just a notification mechanism on top of the existing lease system. No separate oplock subsystem needed — it's a natural extension of ref_mut semantics.

**Oplock Levels:**
- **Level 1 (Exclusive)** → ref_mut with exclusive notification
- **Level 2 (Shared)** → ref with shared notification
- **Batch** → ref_mut with deferred close notification
- **Filter** → ref with minimal notification

**API Translation:**
- `DeviceIoControl(FSCTL_REQUEST_OPLOCK)` → acquire ref_mut with oplock flag
- Oplock break → kernel notification when another process requests access
- `DeviceIoControl(FSCTL_OPLOCK_BREAK_ACKNOWLEDGE)` → downgrade lease level

### Short Filenames (8.3)

**The node-native approach:** Short names are just an attribute, generated on-demand and cached.

```rust
[Program Files]
  win32_path: "/Program Files"
  win32_short_name: "PROGRA~1"  // generated once, cached

[Program Files (x86)]
  win32_path: "/Program Files (x86)"
  win32_short_name: "PROGRA~2"  // collision, increment suffix
```

The short name generation algorithm runs when a node is created with a long name. The result is stored as an attribute. Queries can match against either the long name or short name. No runtime generation, no scanning for collisions — just an indexed attribute.

**Generation Algorithm:**
1. Take first 6 characters of name (uppercase, strip invalid chars)
2. Append `~1`
3. If collision exists, increment suffix (`~2`, `~3`, etc.)
4. Cache result in `win32_short_name` attribute

**API Translation:**
- `GetShortPathName()` → read `win32_short_name` attribute
- `FindFirstFile()` → return both long and short names from attributes
- Path resolution → query matches both `win32_path` and `win32_short_name`

### Volume Shadow Copy (VSS)

**The node-native approach:** Snapshots are just cloning the ownership subgraph at a point in time.

```rust
// Create a snapshot
let snapshot = liblob::create()
    .attr("type", "vss_snapshot")
    .attr("timestamp", now)
    .owner(UNOWNED)
    .build()?;

// Clone all nodes in the volume into the snapshot
for node in volume.all_nodes() {
    let snapshot_node = liblob::clone(node, snapshot)?;
}
```

A VSS snapshot is a node that owns cloned copies of all nodes in the volume at a specific point in time. The clones share data with the originals via copy-on-write (a journal optimization). Accessing previous versions is just querying for snapshot nodes and traversing their owned subgraphs.

This is simpler than NTFS's VSS implementation — snapshots are just owned subgraphs of cloned nodes. The journal handles copy-on-write. No separate snapshot subsystem needed.

**API Translation:**
- `CreateVssSnapshot()` → create snapshot node, clone volume subgraph
- `QueryVssSnapshots()` → query for nodes with `type:vss_snapshot`
- `OpenSnapshotFile()` → resolve path within snapshot's owned subgraph
- Previous Versions UI → enumerate snapshot nodes, show timestamps

**Copy-on-Write Optimization:**
The journal can detect when a node is cloned and share the underlying data blocks until one copy is modified. This makes snapshots space-efficient without requiring kernel changes — it's just a journal optimization.

---

## The Pattern

Every Win32 feature maps to nodes, edges, and attributes:

- **ADS** → owned child nodes with `stream_name`
- **Reparse points** → nodes with `type:reparse_point` and `reparse_tag`
- **Oplocks** → ref_mut leases with notification callbacks
- **Short names** → cached attribute generated at creation
- **VSS** → owned subgraphs of cloned nodes

None of these require new kernel primitives. They're all patterns built on top of the existing node/edge/attribute model. This is the design principle: **always ask "how does this map to nodes?" before implementing anything.**

---

## Implementation Strategy

For each Win32 feature:

1. **Identify the core semantic** — what is this feature actually doing?
2. **Map it to nodes/edges/attributes** — how does it fit the LOBNS model?
3. **Implement in libwin32** — translate Win32 API calls to LOBNS operations
4. **Add kernel support only if necessary** — most features don't need new syscalls

The kernel stays simple. The complexity lives in libwin32, which is userspace and can be iterated on quickly. This is the right architecture.

---

## Why libwin32 Is More Feasible Than Wine

With this approach, libwin32 is more feasible than Wine because:

- **No fighting the underlying OS** — every feature maps naturally to LOBNS
- **No separate subsystems** — registry, VSS, oplocks are all just node patterns
- **Userspace implementation** — most complexity is in libwin32, not the kernel
- **Incremental progress** — each feature can be added independently
- **Architectural alignment** — Win32's object model maps to LOBNS naturally

Wine's 30-year struggle was fighting POSIX. libwin32 won't have that problem. The struggle will be the sheer API surface size, but that's just work, not architectural friction.

---

## Challenges

### 1. API Surface Size
Win32 has ~20,000 APIs. Even with LOBNS's advantages, this is years of work. Prioritization is critical — implement the most commonly used APIs first.

### 2. Undocumented Behavior
Many Win32 APIs have undocumented quirks that applications depend on. Wine has spent 30 years discovering these. libwin32 can learn from Wine's experience but will still encounter edge cases.

### 3. Binary Compatibility
PE loader, relocations, import tables, TLS callbacks — this is complex and must be perfect. Any bugs here break everything.

### 4. Graphics and DirectX
Modern Windows applications expect DirectX or OpenGL. This is a massive undertaking. Likely solution: integrate existing projects like DXVK rather than reimplementing.

### 5. .NET and COM
Many Windows applications use .NET or COM. Supporting these requires substantial additional infrastructure.

---

## Why libwin32 on LOB Is Better Than Wine

**Architectural Alignment**: Win32's object model (handles, named objects, registry) maps naturally to LOBNS. Wine fights POSIX constantly. libwin32 works with the data model.

**Thinner Translation Layer**: Less impedance mismatch means fewer edge cases, fewer bugs, and better performance.

**Native Features**: Registry, file watching, and handle management are native to LOBNS, not emulated.

**Provenance**: LOBNS's provenance tracking makes debugging easier. You can query which process created which nodes, which binary was running, and what DLLs were loaded.

**Security**: LOBNS's ownership model provides structural access control. Win32 security descriptors are emulated for compatibility, but the real enforcement is in the graph.

---

## Realistic Expectations

**Year 10**: Basic Win32 compatibility. Simple CLI and GUI tools work. Some complex applications work with limitations.

**Year 15**: Broad compatibility. Most Windows software runs, though some edge cases remain. Performance is noticeably better than Wine for certain workloads.

**Year 20**: Mature compatibility layer. libwin32 is a viable alternative to Wine, with better performance and fewer bugs for applications that work.

libwin32 will never reach 100% compatibility — Wine hasn't after 30 years. But the architectural advantages mean that applications that do work will work better than on Wine.
