## The Core Idea: One Store, Two Backing Tiers

From the application's perspective there is exactly one node store. You call `create()`, you get a node ID, you use it. Whether that node lives in RAM, on flash, or spans both is the kernel's problem, not the application's. The API is identical either way:

```rust
// Application code — never thinks about where nodes live
let scratch = liblob::create()
    .attr("type", "buffer")
    .data(my_bytes)
    .owner(self_process_id)
    .build()?;

// Use it exactly the same way regardless of where it lives
let r = liblob::ref(scratch)?;
do_something_with(&r);
liblob::drop(scratch)?;
```

The application has no `create_in_memory()` vs `create_on_disk()`. There is just `create()`. This is the right design because the moment you expose that distinction in the API, every application has to think about it, every library has to thread it through, and you've created a two-tier system that leaks implementation details everywhere.

---

## How the Kernel Decides Where a Node Lives

The kernel makes tiering decisions based on node attributes and ownership structure, not on explicit application requests:

```rust
// Kernel-side tiering policy — invisible to applications
fn decide_tier(node: &Node) -> Tier {
    // Ephemeral nodes owned by a process
    // Never need to survive a reboot
    if is_process_owned(node) && !node.attrs.contains("persist") {
        return Tier::Memory;
    }
    
    // Nodes explicitly marked persistent
    if node.attrs.get("persist") == Some("true") {
        return Tier::Disk;
    }
    
    // Nodes not owned by any process — kernel owned
    // These are always persistent
    if node.owner.is_none() {
        return Tier::Disk;
    }
    
    // Default — start in memory, spill to disk under pressure
    Tier::Memory
}
```

The heuristic is simple and correct for most cases:

- Nodes owned by a process are ephemeral by default — they die with the process anyway, no point persisting them
- Nodes with no owner are kernel-owned and always persistent
- A `persist` attribute explicitly opts a node into disk backing

---

## The One Hint the Application Can Give

The only thing an application needs to express is intent — not mechanism:

```rust
// "I want this to survive reboots"
let document = liblob::create()
    .attr("type", "document")
    .attr("persist", "true")   // ← the only hint needed
    .owner(self_process_id)
    .build()?;

// "This is scratch space, never persist it"
let buffer = liblob::create()
    .attr("type", "buffer")
    // no persist attr — kernel treats as ephemeral
    .owner(self_process_id)
    .build()?;
```

Even this hint is optional. The kernel's default heuristic handles the common cases correctly without any hints at all — scratch nodes owned by processes are ephemeral, document nodes that outlive their creator process are persistent. The hint exists for the cases where the heuristic gets it wrong, like a process that creates a node it wants to persist and then immediately transfers ownership.

---

## What the Kernel Actually Does Internally

Internally the kernel has a unified node table that spans both tiers:

```rust
// Kernel internal — never visible to applications
struct NodeEntry {
    node:     Node,
    tier:     Tier,
    dirty:    bool,      // modified since last write to disk
    pinned:   bool,      // locked in RAM, cannot be paged
    lru:      u64,       // for eviction decisions
}

enum Tier {
    Memory,              // RAM only, will not survive reboot
    Disk,                // persisted, RAM copy is a cache
    Spilled,             // was Memory, spilled to disk under pressure
}
```

Every node in the system has an entry in this table regardless of tier. A `Disk` node that is currently accessed lives in RAM as a cache of its on-disk representation — when it is modified the kernel marks it dirty and eventually writes it through to the journal. A `Memory` node never touches the journal. A `Spilled` node started as `Memory` but got evicted to disk under memory pressure — it behaves like a `Disk` node from that point forward except it gets cleaned up when its owner dies.

The application calls `ref(node_id)`. The kernel looks up the entry. If the node is in RAM, it returns immediately. If the node is on disk, it pages it in first, transparently. The application never knows.

---

## Why This Is Better Than Explicit Distinction

Every system that exposes memory vs disk explicitly ends up with the same problem — application code becomes littered with tier-awareness that has nothing to do with what the application is actually trying to do:

```c
// What you want to avoid — tier leaking into application logic
if (needs_persistence) {
    fd = open(path, O_RDWR | O_CREAT);
    write(fd, data, len);
    fsync(fd);  // don't forget this or it might not actually persist
} else {
    ptr = malloc(len);
    memcpy(ptr, data, len);
}
// Now you have two different code paths for the same logical operation
// and you have to track which things are which everywhere
```

In LOB, the same operation with different persistence intent is:

```rust
// Persistent
liblob::create().attr("persist", "true").data(data).build()?;

// Ephemeral  
liblob::create().data(data).build()?;
```

One code path. The rest is the kernel's problem.

---

## The Edge Cases

### What About Performance-Critical Scratch Space?

An application doing heavy computation might want to guarantee a node never touches disk even under memory pressure — a video decoder's frame buffer, for example. The `pinned` concept handles this:

```rust
// Guarantee this node stays in RAM
// Kernel error if it can't honour the pin
liblob::create()
    .attr("type", "frame_buffer")
    .attr("pin", "true")
    .owner(self_process_id)
    .build()?;
```

This is the one case where the application legitimately cares about physical location — not for persistence semantics but for performance guarantees. It's still not `create_in_memory()` — it's expressing a performance requirement and letting the kernel decide how to honour it.

### What About Explicit Flush?

Sometimes an application wants to know its persistent nodes are actually on disk before proceeding — after a save operation, for example. A single syscall handles this:

```rust
// Ensure this node and its owned subtree are durably written
liblob::sync(document_id)?;
// Returns only after the journal commit record is on disk
// Equivalent to fsync() but for a node subtree, not a file descriptor
```

This is the LOB equivalent of `fsync()` — explicit durability confirmation when the application needs it. It is separate from the persistence decision, which was made at creation time.

### What Happens to Spilled Nodes When the Owner Dies?

When a process dies, the kernel drops its process node, which cascades to all owned nodes. For `Memory` nodes this is just freeing RAM. For `Spilled` nodes the kernel also removes them from disk — they were ephemeral, they just temporarily lived on disk due to memory pressure, and their intended lifetime was always "until the owner dies." The journal entry for a spilled node's creation is never promoted to the main node region — it stays in the journal as a temporary entry and is discarded on the next compaction after the owner dies.

---

## The Mental Model

Think of it like CPU caches. You do not write code that explicitly manages L1 vs L2 vs L3 cache vs RAM. You write code that accesses memory. The hardware decides what lives where based on access patterns. You can give hints (prefetch instructions, cache line alignment) but the mechanism is transparent.

LOBNS does the same thing for RAM vs disk. You write code that accesses nodes. The kernel decides what lives where based on ownership structure, access patterns, and memory pressure. You can give hints (`persist`, `pin`) but the mechanism is transparent.

The line between memory and disk being blurry is not a problem to solve. It is the correct design.