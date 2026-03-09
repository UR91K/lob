If LOBNS nodes are the primary data structure rather than heap allocations, you get some unusual properties:

**Persistence by default changes everything:**

```rust
// Traditional approach - everything in RAM
let mut buffer = String::new();
editor.insert_text(&mut buffer, "hello");
// Lost on crash

// LOBNS approach - working directly on persistent nodes
let buffer = liblob::create()
    .attr("type", "text_buffer")
    .owner(self_process_id)
    .build()?;

liblob::ref_mut(buffer, |data| {
    data.insert_str(cursor_pos, "hello");
})?;
// Survives crash, no explicit save needed
```

Every keystroke is already on disk (modulo journal buffering). Undo/redo becomes "create a new revision node" rather than maintaining in-memory state. The editor never has an "unsaved changes" state because there's no distinction between working memory and storage.

**Structural sharing becomes natural:**

```rust
// A build system doesn't copy source files into a build directory
// It just creates nodes that ref the sources

let build = liblob::create()
    .attr("type", "build")
    .owner(self_process_id)
    .build()?;

for src in sources {
    liblob::create_edge(build, src, EdgeKind::Ref)
        .attr("role", "input")?;
}

let output = compile(build)?;
liblob::create_edge(build, output, EdgeKind::Own)
    .attr("role", "output")?;

// The build node owns its outputs but only refs its inputs
// Dropping the build cleans up artifacts but leaves sources alone
```

No copying, no temp directories, no cleanup scripts. The ownership model expresses exactly what you mean.

**IPC becomes "just share a node":**

```rust
// Process A creates a work queue
let queue = liblob::create()
    .attr("type", "queue")
    .data(serialize_queue(&[]))
    .build()?;

// Hand it to process B
send_to_process(process_b, queue)?;

// Both processes can ref_mut it (with kernel arbitration)
// No sockets, no pipes, no serialization overhead
// Just two processes with refs to the same node
```

The kernel already handles exclusive access through ref_mut leases. You don't need a separate IPC mechanism — shared nodes are IPC.

**Incremental computation falls out naturally:**

```rust
// A compiler can check content_hash of source nodes
// Only recompile if the hash changed since last build

for src in sources {
    let last_hash = build_cache.get(src)?;
    let current_hash = liblob::get_node(src)?.content_hash;
    
    if last_hash != current_hash {
        recompile(src)?;
    }
}
```

Content addressing is built in. You don't need a separate build cache or timestamp tracking. The node store is the build cache.

**The interesting tension:**

You want tools to lean on LOBNS, but you also need them to be fast. Every node operation is a syscall. If a text editor creates a new node for every character typed, that's a lot of syscalls. You'll probably want some buffering:

```rust
// Maybe the editor keeps a small in-memory buffer
// and flushes to the node periodically or on idle

struct Editor {
    node: NodeId,
    dirty_buffer: String,
    last_flush: Instant,
}

impl Editor {
    fn insert(&mut self, pos: usize, text: &str) {
        self.dirty_buffer.insert_str(pos, text);
        
        if self.dirty_buffer.len() > 4096 || self.last_flush.elapsed() > Duration::from_secs(1) {
            self.flush()?;
        }
    }
    
    fn flush(&mut self) -> Result<()> {
        liblob::ref_mut(self.node, |data| {
            *data = self.dirty_buffer.clone().into_bytes();
        })?;
        self.last_flush = Instant::now();
        Ok(())
    }
}
```