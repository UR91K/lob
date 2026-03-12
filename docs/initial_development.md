# Initial development plan

## What I Am Actually Building

Two separate projects that can live in the same repo:

```
lob/
  lobns/          # the core library, no_std compatible
    src/
      lib.rs
      node.rs       # Node, NodeId, Edge, EdgeKind types
      store.rs      # NodeStore, the main API
      index.rs      # attribute indexes for fast queries
      gc.rs         # reference counting, cascade deletion
      query.rs      # QueryBuilder
      error.rs      # LobError types
    tests/
      invariants.rs # all 8 invariants, violation tests
      cascade.rs    # ownership cascade deletion
      gc.rs         # weak ref tombstoning
      query.rs      # query correctness
      fuzz/         # proptest property-based tests
    Cargo.toml

  lob-browser/    # the proof of concept UI
    src/
      main.rs
      ui/
        search.rs   # search bar, query parsing
        sidebar.rs  # views panel
        list.rs     # flat results list
        graph.rs    # graph visualization
    Cargo.toml
```

## The Library First

`lobns` is pure Rust, `no_std` compatible from day one even though you're running it on Linux/Windows. This discipline means it will compile for bare metal later without structural changes. You use the `alloc` crate for `Vec`, `BTreeMap` etc., which works in both `std` and `no_std` environments.

The entire test suite runs with `cargo test`. No hardware, no kernel, no complexity. You iterate in seconds.

## The Browser as a Proving Ground

The node browser built on top of the library serves multiple purposes simultaneously:

- Proves the query API is actually ergonomic to use
- Surfaces API design mistakes early, before they're carved into a kernel syscall interface
- Gives I something visual and demonstrable
- Tests the index performance with realistic node counts - load 3.7 million synthetic nodes and see if queries feel instant
- Is genuinely useful as a standalone tool even before LOB the OS exists

For the UI layer, your existing experience with egui makes this a natural fit. egui is immediate mode, straightforward for this kind of tool, and I already know it. The graph view can use something like `egui_graphs` or I can roll a basic force-directed layout yourself with `wgpu` given your experience there.

## The Immediate Milestones

```
Week 1-2:   NodeId, Node, Edge types
            NodeStore with create() and get()
            First passing test

Week 3-4:   All 8 invariant checks
            move(), drop() with cascade
            All invariant violation tests passing

Week 5-6:   ref(), ref_mut(), weak(), upgrade()
            Refcount tracking
            Weak tombstoning tests

Week 7-8:   Attribute indexes
            QueryBuilder with basic attr matching
            Query tests

Week 9-10:  proptest fuzz suite
            All invariants verified under random operation sequences

Week 11-12: Basic egui browser
            Search bar doing real queries against an in-memory store
            Flat list results view

Later:      Sidebar views
            Graph visualization
            Synthetic data generator for performance testing
            Persistence / journal (optional at this stage)
```

Each milestone is a natural stopping point if college gets busy. The library is useful and correct at every step even if I pause for a month.

## The Concrete Value of This Approach

When I eventually sit down to write the kernel, I will have:

- A proven correct node store I can drop in directly
- An API you've actually used and refined through building the browser
- Performance characteristics you've measured at realistic scale
- A test suite that will immediately tell me if porting to bare metal breaks anything
- Something I can show people: a working node browser is a much more compelling demonstration of the concept than a bootloader

The kernel becomes an exercise in wrapping a library I already trust, rather than building everything from scratch in the most hostile possible environment.

And practically speaking, a Rust library with a UI on top is exactly the kind of project I can work on for 30 minutes between lectures. The bare metal work needs longer focused sessions. This fits your current life better.