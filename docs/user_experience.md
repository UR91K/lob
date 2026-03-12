# User Experience

The primary user interface for LOB is a flat node browser with no directory tree to navigate — only a search bar and a sidebar of saved views. A second tab renders the same data as a visual graph showing ownership islands, relationships, and provenance chains.

---

## The Node Browser

The browser presents a flat list of nodes matching the current query. There is no directory tree, no navigation hierarchy. You find data by what it is, not where it is.

### The Search Bar

```
notes                            fuzzy name and content match
type:image                       exact attribute match
type:image user:alice            multiple attributes, implicitly ANDed
"meeting notes"                  phrase match
created_at:>2024-01-01           kernel field with comparison
created_by_binary:[node-id]      everything a specific program ever created
created_by_session:abc123        everything from a specific session
/^report_\d{4}/                  regex against name attribute
type:image,video                 OR within a single attribute
!type:view                       negation
```

The query language addresses both kernel fields and application attrs with identical syntax. The query engine knows which terms map to kernel struct fields and which map to the application attr map — the user never needs to care about this distinction.


### Query Examples

**Find all images created by alice:**
```
type:image user:alice
```

**Find everything firefox has ever created:**
```
created_by_binary:[firefox-node-id]
```

**Find recent documents:**
```
type:document created_at:>2024-01-01
```

**Find work-related notes:**
```
type:text tag:work
```

**Find nodes modified in the last week:**
```
modified_at:>2024-03-02
```

**Find all nodes from a specific session:**
```
created_by_session:abc123
```

---

## Views

A view is a node in LOBNS with `type:view` and a serialized query as its data. Views are first-class — owned, shared, queried like anything else.

```rust
// Create a view
let view = liblob::create()
    .attr("type", "view")
    .attr("name", "Work Documents")
    .data(b"type:document tag:work")
    .owner(UNOWNED)
    .build()?;
```

Views appear in the sidebar. Clicking a view executes its query and displays the results. Views are live — the result set updates automatically as nodes are created, modified, or deleted.

### Push Notifications

A process can hold a Ref to a view node and receive push notifications when the result set changes:

```rust
let view_ref = liblob::ref(view_id)?;
liblob::subscribe(view_ref, |event| {
    match event {
        ViewEvent::NodeAdded(id) => println!("New node: {}", id),
        ViewEvent::NodeRemoved(id) => println!("Node removed: {}", id),
        ViewEvent::NodeModified(id) => println!("Node changed: {}", id),
    }
})?;
```

The browser updates in real time with no polling.

---

## The Graph View

A second tab renders the same data as a visual graph — nodes as circles, edges as lines, ownership islands as natural clusters.

### Visual Encoding

- **Nodes** — circles, sized by data size or refcount
- **Ownership edges** — solid lines, form tree structures
- **Ref edges** — dashed lines, show shared access
- **Weak edges** — dotted lines, show provenance
- **Node type** — determines color (blue for executables, green for documents, etc.)

### What the Graph Shows

The graph view makes immediately visible things no conventional OS can show:

- **Blast radius** — what dies if you drop this node?
- **Survival** — what survives if you uninstall this package?
- **Provenance chain** — trace any node back to the binary that created it
- **Anomalous behaviour** — a process holding edges to nodes it has no legitimate reason to access is structurally visible

### Example: Package Uninstall

```
Before uninstall:

[pkg: firefox] ──own──► [bin: firefox]
               ──own──► [bin: crashreporter]
               ──own──► [res: icons]

[profile: alice] ──weak──► [bin: firefox]
                 ──ref──► [bookmarks]

After drop(pkg):

[profile: alice] ──weak──► [tombstone]
                 ──ref──► [bookmarks]
```

The graph shows that the profile survives because it was never owned by the package. The weak edge becomes a tombstone, which is expected and correct.

---

## Names Are Artificial

In a conventional filesystem, a filename is fundamental — it is how the filesystem locates a node. In LOBNS, identity is the node ID. A name is just an attribute, no different from `type` or `created_at`. There is nothing structurally special about it.

Nameless nodes are the natural default, not a special case. A process that needs scratch space creates a nameless node it owns. When the process exits, the cascade reaches the scratch node and it disappears. No name to invent, no `/tmp` collision, no cleanup code required.

### Naming Conventions

Applications are free to use whatever naming conventions make sense:

```rust
// Traditional filename
attr("name", "report.pdf")

// Semantic attributes instead of name
attr("type", "report")
attr("year", "2024")
attr("quarter", "Q1")

// No name at all
// (found by query: type:scratch owner:[process-id])
```

The browser can display nodes with or without names. Queries work regardless of whether a name exists.

---

## No Directory Navigation

There is no "current directory" concept. You do not `cd` into a location. You query for what you want and the browser shows it.

This eliminates entire classes of problems:

- No "where did I save that file?"
- No deeply nested directory structures to navigate
- No ambiguity about relative vs absolute paths
- No need to remember directory hierarchies

Everything is found by query. The query is the interface.

---

## Vocabulary

The vocabulary deliberately separates itself from POSIX conventions:

| POSIX | LOB |
|---|---|
| file | node |
| directory | view or owned subgraph |
| path | node ID or query |
| file descriptor | ref handle |
| inode | node |
| symlink | — not needed |
| hardlink | — not needed |
| filesystem | LOBNS |

This is not just terminology — it reflects a fundamentally different model. There are no files, no directories, no paths, no mounts. There are only nodes, edges, and queries.

---

## Example Workflows

### Creating a Document

```rust
// No need to choose a location or filename upfront
let doc = liblob::create()
    .attr("type", "document")
    .owner(self_process_id)  // ephemeral while editing
    .build()?;

// Edit...
liblob::ref_mut(doc, |data| {
    data.extend_from_slice(b"Document content...");
})?;

// Save — add metadata and move to unowned
liblob::set_attr(doc, "name", "Q1 Report")?;
liblob::set_attr(doc, "tag", "work")?;
liblob::move(doc, UNOWNED)?;

// Now found by query: type:document tag:work
```

### Finding Related Nodes

```rust
// Find all nodes created by the same binary as this node
let binary = liblob::get_attr(node, "created_by_binary")?;
let related = liblob::query(&format!("created_by_binary:{}", binary))?;

// Find all nodes that ref this node
let refs = liblob::query(&format!("refs:{}", node))?;

// Find all nodes owned by this node
let owned = liblob::query(&format!("owner:{}", node))?;
```

### Organizing with Views

```rust
// Create a view for work documents
let work_view = liblob::create()
    .attr("type", "view")
    .attr("name", "Work")
    .data(b"type:document tag:work")
    .owner(UNOWNED)
    .build()?;

// Create a view for recent images
let images_view = liblob::create()
    .attr("type", "view")
    .attr("name", "Recent Images")
    .data(b"type:image created_at:>2024-01-01")
    .owner(UNOWNED)
    .build()?;
```

Views appear in the sidebar and update automatically as nodes are created or modified.

---

See also:
- [README.md](README.md) — Overview and quick start
- [node_store.md](node_store.md) — Node and edge definitions
- [security.md](security.md) — Query scoping and access control
- [libposix.md](libposix.md) — POSIX path emulation
