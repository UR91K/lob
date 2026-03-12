# Implementation

LOB is built in phases, with each phase being a complete usable system before the next begins. This document describes the testing strategy, development phases, and project structure.

---

## Testing Strategy

LOBNS takes direct inspiration from SQLite's approach to correctness - 100% branch coverage, every possible failure point tested, a test suite orders of magnitude larger than the library itself. LOB applies the same discipline.

### Invariant Checking

In debug builds, every mutation runs a full graph-wide invariant check immediately after it completes. Violations are caught at the exact operation that caused them, not discovered later through mysterious symptoms.

```rust
#[cfg(debug_assertions)]
fn verify_invariants(store: &NodeStore) -> Result<()> {
    for node in store.iter() {
        // Invariant 1: Every node has exactly one owner, or is explicitly unowned
        // (already enforced by Option<NodeId>)
        
        // Invariant 2: Cannot move a node while any Ref edge points to it
        if node.refcount > 0 {
            assert!(node.owner.is_some(), "Node {} has refs but no owner", node.id);
        }
        
        // Invariant 3: Cannot move a node while any ref_mut lease is active
        if node.ref_mut {
            assert!(node.owner.is_some(), "Node {} has ref_mut but no owner", node.id);
        }
        
        // Invariant 4: Only one ref_mut lease can exist for a node at any time
        // (enforced by bool flag)
        
        // Invariant 5: Ownership edges cannot form cycles
        verify_no_ownership_cycles(store, node.id)?;
        
        // Invariant 6: Dropping an owner cascades to all owned nodes deterministically
        // (tested separately in cascade tests)
        
        // Invariant 7: Weak edges never prevent deletion
        for edge in &node.edges {
            if edge.kind == EdgeKind::Weak {
                // Weak edges can point to tombstones
                let _ = store.get_node(edge.target);  // may fail, that's ok
            }
        }
        
        // Invariant 8: A node's data cannot be mutated without a ref_mut lease
        // (enforced by API design)
    }
    
    Ok(())
}
```


### Property-Based Testing

`proptest` generates thousands of random operation sequences automatically. After every single operation in every sequence, all eight invariants are verified. `proptest` finds and shrinks the minimal failing sequence automatically.

```rust
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Operation {
    Create { owner: Option<NodeId> },
    Move { node: NodeId, to: Option<NodeId> },
    Ref { node: NodeId },
    RefMut { node: NodeId },
    Drop { node: NodeId },
    Weak { node: NodeId },
}

proptest! {
    #[test]
    fn random_operations_preserve_invariants(ops in prop::collection::vec(any::<Operation>(), 1..100)) {
        let mut store = NodeStore::new();
        
        for op in ops {
            // Execute operation (may fail, that's ok)
            let _ = execute_operation(&mut store, op);
            
            // Invariants must hold after every operation
            verify_invariants(&store).unwrap();
        }
    }
}
```

This catches edge cases that manual testing would never find. `proptest` has found bugs in production Rust code that existed for years.

### Fault Injection

The journal layer exposes a `StorageBackend` trait. In tests, a `FaultInjectingBackend` simulates power loss after any specific write. Every operation is tested with power loss injected at every possible write boundary, then the simulated system boots from the journal and all invariants are verified after recovery.

```rust
trait StorageBackend {
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<()>;
    fn read(&mut self, offset: u64, len: usize) -> Result<Vec<u8>>;
    fn sync(&mut self) -> Result<()>;
}

struct FaultInjectingBackend {
    inner: Box<dyn StorageBackend>,
    fail_after: Option<usize>,
    write_count: usize,
}

impl StorageBackend for FaultInjectingBackend {
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<()> {
        self.write_count += 1;
        if Some(self.write_count) == self.fail_after {
            return Err(Error::PowerLoss);
        }
        self.inner.write(offset, data)
    }
    
    // ... other methods
}

#[test]
fn test_crash_recovery() {
    for fail_point in 0..100 {
        let mut backend = FaultInjectingBackend::new(fail_point);
        let mut store = NodeStore::new_with_backend(backend);
        
        // Perform some operations
        let node = store.create(None).unwrap();
        store.set_data(node, b"test data").unwrap();
        
        // Simulate crash and recovery
        let recovered = NodeStore::recover_from_journal(backend).unwrap();
        
        // All invariants must hold after recovery
        verify_invariants(&recovered).unwrap();
    }
}
```

### Mutation Testing

`cargo-mutants` automatically modifies the source and verifies that every mutation causes a test failure. Any mutation that passes is a real gap in coverage.

```bash
cargo install cargo-mutants
cargo mutants
```

This finds untested code paths and weak assertions. If changing `>` to `>=` doesn't break any tests, the tests are insufficient.

### The Node Store as a Pure Library

The node store is a `no_std` Rust library that runs with `cargo test` on a development machine, independent of any hardware or kernel. The node store is proven correct in safe Rust before it ever runs on bare metal.

```rust
// lobns/src/lib.rs
#![no_std]

extern crate alloc;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;

pub struct NodeStore {
    nodes: BTreeMap<NodeId, Node>,
    next_id: u64,
}

impl NodeStore {
    pub fn new() -> Self {
        NodeStore {
            nodes: BTreeMap::new(),
            next_id: 1,
        }
    }
    
    // ... all operations
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_node() {
        let mut store = NodeStore::new();
        let node = store.create(None).unwrap();
        assert_eq!(node, NodeId(1));
    }
    
    // ... thousands of tests
}
```

---

## Implementation Phases

LOB is built in phases, with each phase being a complete usable system before the next begins.

### Phase 0 - Proof of Concept on Linux/Windows

**Goal:** Prove the model is correct and the API is ergonomic before any hardware is involved.

**Deliverables:**
- LOBNS node store as a pure `no_std` Rust library
- Comprehensive test suite (property-based fuzzing, fault injection, 100% branch coverage)
- Node browser as an egui application
- CLI tool for creating, querying, and manipulating nodes
- Disk writer targeting a real disk image or partition
- FUSE mount on Linux for testing POSIX compatibility

**Status:** In progress. The node store is implemented with full invariant checking. Property-based tests are running. Fault injection is implemented.

### Phase 1 - Bare Metal Kernel

**Goal:** Boot on target ARM hardware and run the node store in RAM.

**Deliverables:**
- Boot on target ARM hardware (Raspberry Pi 4 or similar)
- UART output as the first milestone
- Memory allocator, interrupt handling, timer
- The node store running in RAM only
- A basic syscall layer wrapping LOBNS operations

**Milestones:**
1. UART "Hello, world!"
2. Memory allocator working
3. Node store running in RAM
4. First syscall (`create_node`) working

### Phase 2 - Interactive System

**Goal:** A demonstrable system with scheduler, shell, and userspace API.

**Deliverables:**
- Scheduler and context switching
- The LOB shell querying the in-RAM node store
- liblob userspace API
- Basic process management (spawn, exit, wait)
- IPC via shared nodes

**Milestones:**
1. Two processes running concurrently
2. Shell accepting commands
3. Shell querying nodes and displaying results
4. Process creating nodes and querying them

### Phase 3 - Persistence

**Goal:** Crash-consistent writes to storage, boot from persisted node store.

**Deliverables:**
- The journal layer, crash-consistent writes to storage
- On-disk node serialization
- Boot from a persisted node store
- Crash recovery tested exhaustively with fault injection

**Milestones:**
1. First successful write to disk
2. First successful boot from disk
3. Crash recovery working
4. All fault injection tests passing

### Phase 4 - Native Userspace

**Goal:** A complete self-hosting system with native applications.

**Deliverables:**
- The node browser running natively on LOB
- Native applications (text editor, file manager, terminal)
- Package manager
- Network stack
- Device drivers (USB, network, display)

**Milestones:**
1. Node browser running natively
2. Text editor creating and saving nodes
3. Package manager installing packages
4. Network stack working

### Phase 5 - POSIX Compatibility

**Goal:** Run legacy software without modification.

**Deliverables:**
- libposix, musl libc integration
- ELF loader, dynamic linker
- Enough syscall coverage to run simple C programs
- Progressing toward complex software (gcc, python, etc.)

**Milestones:**
1. "Hello, world!" C program running
2. coreutils (ls, cat, grep) running
3. gcc compiling C programs
4. Python interpreter running

---

## Project Structure

LOB follows a BSD-style monorepo - the entire system in one repository, one build system, one release. LOBNS is not a swappable component as filesystems are in Linux. It is the identity of the OS.

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
  tests/        # integration tests, fault injection, property tests
```

Nothing in this structure uses the word "file" except `libposix`, which is exactly where the POSIX concept belongs - quarantined in the compatibility layer, not present in the native system.

---

## Development Workflow

### Running Tests

```bash
# Unit tests
cargo test --package lobns

# Property-based tests (slow)
cargo test --package lobns --release -- --ignored

# Fault injection tests (very slow)
cargo test --package lobns --release -- --ignored fault_injection

# Mutation testing
cargo mutants --package lobns
```

### Building the Kernel

```bash
# Build for target hardware
cargo build --target aarch64-unknown-none --release

# Create bootable image
./tools/make-image.sh target/aarch64-unknown-none/release/kernel
```

### Running in QEMU

```bash
# Boot in QEMU
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -m 1G \
  -kernel target/aarch64-unknown-none/release/kernel \
  -serial stdio \
  -display none
```

### FUSE Mount (Phase 0 only)

```bash
# Mount LOBNS as a FUSE filesystem for testing
cargo run --bin lobns-fuse -- /mnt/lob

# Now you can use standard tools
ls /mnt/lob
cat /mnt/lob/some-node
```

---

## Code Quality Standards

- **100% branch coverage** on the node store
- **No unsafe code** except in the kernel's hardware abstraction layer
- **All public APIs documented** with examples
- **Property-based tests** for all core operations
- **Fault injection tests** for all journal operations
- **Mutation testing** to verify test quality
- **Clippy clean** with no warnings
- **rustfmt** enforced in CI

---

## Contributing

LOB is an experimental research OS. Contributions are welcome but should align with the core design principles:

- Ownership semantics are non-negotiable
- The eight invariants must be preserved
- Simplicity over features
- Correctness over performance (but performance matters)
- No POSIX in the kernel

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

---

See also:
- [README.md](README.md) - Overview and quick start
- [node_store.md](node_store.md) - Core data structures
- [security.md](security.md) - Security model
- [reproducibility.md](reproducibility.md) - Testing reproducibility
