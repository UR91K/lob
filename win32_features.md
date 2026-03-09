That's the right mindset. Let me apply it to the features I said you were missing:

## Alternate Data Streams

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

## Reparse Points

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

## Opportunistic Locks

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

## Short Filenames (8.3)

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

## Volume Shadow Copy

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

## The Pattern

Every Win32 feature you mentioned maps to nodes, edges, and attributes:

- **ADS** → owned child nodes with `stream_name`
- **Reparse points** → nodes with `type:reparse_point` and `reparse_tag`
- **Oplocks** → ref_mut leases with notification callbacks
- **Short names** → cached attribute generated at creation
- **VSS** → owned subgraphs of cloned nodes

None of these require new kernel primitives. They're all patterns built on top of the existing node/edge/attribute model. This is exactly what you meant by "lean on the node-based data structure."

## The Implementation Strategy

For each Win32 feature:

1. **Identify the core semantic** — what is this feature actually doing?
2. **Map it to nodes/edges/attributes** — how does it fit the LOBNS model?
3. **Implement in libwin32** — translate Win32 API calls to LOBNS operations
4. **Add kernel support only if necessary** — most features don't need new syscalls

The kernel stays simple. The complexity lives in libwin32, which is userspace and can be iterated on quickly. This is the right architecture.

## What This Means for the Timeline

With this approach, libwin32 is more feasible than Wine because:

- **No fighting the underlying OS** — every feature maps naturally to LOBNS
- **No separate subsystems** — registry, VSS, oplocks are all just node patterns
- **Userspace implementation** — most complexity is in libwin32, not the kernel
- **Incremental progress** — each feature can be added independently

The timeline in your document is achievable if you maintain this discipline: **always ask "how does this map to nodes?" before implementing anything.**

Wine's 30-year struggle was fighting POSIX. You won't have that problem. Your struggle will be the sheer API surface size, but that's just work, not architectural friction.