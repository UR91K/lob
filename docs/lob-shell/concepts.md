# lob-shell Concepts

> **See also:** [Quick Reference](./reference.md) — all commands and flags  
> **See also:** [Cookbook](./cookbook.md) — piping, scripting, real-world workflows

---

## The Node Store

Everything in LOB is a **node** — files, processes, packages, configuration, executables. There is no filesystem hierarchy. Instead, nodes exist in a flat store and are connected by typed edges. Discovery happens through queries, not navigation.

Every node has:

- A unique numeric **ID** assigned at creation
- An optional **name** attribute (not guaranteed unique)
- A set of **attributes** (key-value pairs, including `type`, `data`, `format`, etc.)
- A **refcount** tracking how many active Ref edges point to it
- **Provenance** — which process, binary, and user created it
- Timestamps for creation, modification, and access

---

## Ownership

Every node is either **owned** by exactly one other node, or it is **unowned**.

Ownership is the primary relationship in the store. It determines what happens when a node is dropped — dropping a node cascades to all nodes it owns, recursively. This means ownership forms a tree, and dropping the root of that tree cleans up everything beneath it.

### Owned nodes

A node owned by another node lives and dies with its owner. When the owner is dropped, the owned node is dropped too, regardless of its refcount or other edges. Processes own the nodes they create during their lifetime. Packages own their binaries and assets.

### Unowned nodes

An unowned node has no parent in the ownership tree. It persists until explicitly dropped. Unowned nodes are automatically **journaled** — their data survives a system restart. This is how you persist data beyond the lifetime of the process that created it.

```shell
move 1 unowned       # detach from current owner, make persistent
clone 1 unowned      # duplicate and persist the clone
```

### Ownership and refcount interaction

A node can have active Ref edges (refcount > 0) and still be dropped if its owner is dropped — the cascade does not check refcount. Ref edges track borrowing, not survival. If you need a node to outlive its owner, move it to `unowned` first.

---

## Edge Types

Nodes are connected by three kinds of edges. Each serves a distinct purpose.

### Own edges

Own edges define the ownership tree. Every owned node has exactly one incoming Own edge from its owner. Own edges are created implicitly when a node is created with an owner, and destroyed when the node is moved or dropped.

Own edges are never created manually — they are managed by `move`, `clone`, and `drop`.

### Ref edges

A Ref edge is a **borrow**. It says: "I am currently using this node." Ref edges increment the target's refcount. A node with refcount > 0 cannot be dropped directly — you must first drop or release all Ref edges pointing to it.

Ref edges are the mechanism for safe concurrent access. The `edit` command acquires a special exclusive Ref (`ref_mut`) that blocks other writers.

```shell
ref 1 2              # node 1 borrows node 2
drop 2               # Error: refcount is 1
unlink -t ref 1 2    # release the borrow
drop 2               # now succeeds
```

### Weak edges

A Weak edge is an **observation**. It does not affect refcount, and it does not prevent deletion. If the target node is dropped, the weak edge becomes a **tombstone** — the edge still exists in the graph, but it points to a node that no longer has data.

Weak edges are useful for soft references: caches pointing at source data, a document tracking the binary that created it, a backup index pointing at originals. If the target disappears, the weak edge tells you it happened rather than silently vanishing.

```shell
weak 1 2             # node 1 weakly observes node 2
drop 2               # succeeds, node 1's edge becomes tombstone
upgrade 1 2          # Error: target is tombstone
```

---

## Refcount

A node's **refcount** is the number of active Ref edges pointing to it. It is not a general reference count for ownership — Own edges do not affect refcount.

Refcount governs whether a node can be directly dropped:

- Refcount 0 → can be dropped (if you own it)
- Refcount > 0 → cannot be dropped directly; active borrows must be released first

The `--in-use` / `-iu` query flag finds nodes with refcount > 0 — useful for seeing what is currently active on the system.

---

## Name Resolution

The `@` prefix resolves a node by reference rather than by result number.

| Syntax | Resolves by |
|--------|-------------|
| `@12844` | Exact numeric ID — always unambiguous |
| `@firefox` | `name` attribute |
| `@firefox:package` | `name` and `type` attributes |

Name resolution is **unambiguous when exactly one node matches**. If multiple nodes share a name, the shell presents a numbered disambiguation prompt inline:

```shell
>> drop @firefox
Error: @firefox is ambiguous - 3 nodes match name:firefox
1 @8291 firefox-bin | executable
2 @7819 firefox     | package
3 @9285 firefox     | process
Which would you like to drop? >> 2
```

Adding a type hint narrows the match before prompting. If the hint still matches multiple nodes, the prompt appears with the filtered set.

Any command that accepts a node reference uses this same resolution behavior — it is not specific to any individual command. After disambiguation, the resolved node becomes the context for subsequent `lqr` operations.

---

## Query Context

The shell maintains a **current context**: the result set of the last query. Result numbers (`1`, `2`, `3`) are resolved against this context.

`lqr` operates on the current context instead of the full node store. If no context exists, `lqr` returns an error.

Context is replaced each time a new `qr` runs, or updated in place by traversal operators and `lqr` chains.

---

## Traversal and the Graph

Traversal operators walk edges from the current result set, replacing or combining it with what they find. This lets you navigate the ownership and reference graph without knowing node IDs in advance.

`.o`, `.r`, and `.w` walk one hop along Own, Ref, or Weak edges respectively. The `+` suffix walks until the graph is exhausted (one or more hops). The `*` suffix does the same but includes the starting set.

Set operators (`|.o`, `&.o`, `-.o`) perform union, intersection, and difference between the current set and the traversal result, rather than replacing the set outright.

These compose naturally:

```shell
# Start at alice's nodes, expand the full ownership tree,
# then keep only nodes that have active Ref edges
qr -u alice |.o+ &.r
```

---

## Provenance

Every node records how it came to exist:

- **created_by_process** — the process instance that created it (often a tombstone by the time you check)
- **created_by_binary** — the binary that process was running
- **created_by_user** — the user who ran that process

This chain extends upward: you can trace a binary back to the package manager that installed it, and from there to the user who ran the install. The `trace` command walks this chain and displays it in full, marking tombstoned entries.

Provenance is immutable — it is set at creation and cannot be changed.

---

## Node Lifecycle

```
         new / clone
              │
              ▼
         [created]
              │
    ┌─────────┴─────────┐
    │                   │
  owned             unowned
    │               (journaled,
    │                persistent)
    │
    ▼
  [in use]  ◄──── Ref edges increment refcount
    │
    ▼
  dropped ──► cascade drops all owned children
```

A node moves from owned to unowned via `move ... unowned`. It can be re-owned by moving it to another node. Dropping a node while it has active Ref edges requires first releasing those borrows — the shell will tell you which nodes hold them.

---

## Ephemerality and Journaling

By default, nodes created by a process are **ephemeral** — they exist in memory and are cleaned up when the owning process exits or is dropped. This is appropriate for most working data: editor buffers, process state, temporary computations.

Unowned nodes are **journaled**: their data is written to persistent storage and survives restarts. When you explicitly persist something — moving it to `unowned`, installing a package, saving a document — it becomes journaled.

You can see the split in `nsstats`:

```shell
>> nsstats
  Ephemeral nodes: 1,284
  Journaled nodes: 46,007
```