# LOB Shell Design

The LOB shell (known simply as `shell`) is a query-first interface to the node store. There is no "current directory" - instead, commands operate on query results, node IDs, or the entire system. When a query is made, the last query is stored as the current context (similar to being in a directory) and the list numbers are resolved to actual ids when referenced. 

---

## Core Philosophy

1. **Query is the primitive** - Discovery happens through queries, not navigation
2. **Results are numbered** - Easy reference without typing node IDs
3. **Context is explicit** - The prompt shows your identity and machine, not a fake path
4. **Node IDs are first-class** - Direct reference via `@id` syntax
5. **Syscall names are commands** - `move`, `clone`, `ref`, `drop`, `weak`, `upgrade` match the kernel API
6. **Composable** - Commands can be piped and chained

---

## The `query` Command

The `query` or `qr` command queries the entire system for matching nodes.

### Flags

```
--id -i <node-id>                    Query by exact node ID
--owner-id -o <node-id>              Nodes owned by this node
--reference-from-id -r @<id/name>    Nodes with Ref edge from this node
--weak-from-id -w @<id/name>         Nodes with Weak edge from this node

--size -s <size-spec>                Data size: '1KB', '1-4KB', '8MB', '12-14GB', '1KB-14MB'
--reference-count -rc <count>        Nodes with exactly this refcount
--in-use -iu                         Nodes with refcount > 0 (flag)

--created -c <date/range>            Creation timestamp
--modified -m <date/range>           Modification timestamp  
--accessed -ac <date/range>          Access timestamp

--process -p @<id/name>              Created by this process instance
--binary -b @<id/name>               Created by this binary
--user -u <username>                 Created by this user

--attributes -a <key:value,...>      Match attributes (type:document, name:draft, etc)

--show-id -sid                       Show canonical node IDs alongside names (name@id)
--only-id -oid                       Show only canonical node IDs (name becomes attribute)
```

### Date/Range Syntax

```
2024-03-11                  Exact date
2024-03-11T14:30:00        Exact timestamp
>2024-03-01                After this date
<2024-03-01                Before this date
2024-03-01..2024-03-11     Range (inclusive)
1d                          Last 24 hours
7d                          Last 7 days
30d                         Last 30 days
```

### Examples

```shell
# Find all documents (default display - names only, or node ID if unnamed)
alice (lobbox1) >> qr -a type:document
1 draft      | document utf8 markdown | 2.4KB | mod 2h
2 notes      | document utf8 markdown | 8.1KB | mod 1d
3 report     | document binary pdf    | 1.2MB | mod 3d
4 (@12847)   | document utf8          | 4.2KB | mod 6h

# Show canonical node IDs alongside names
alice (lobbox1) >> qr -a type:document -sid
1 draft@12844      | document utf8 markdown | 2.4KB | mod 2h
2 notes@12845      | document utf8 markdown | 8.1KB | mod 1d
3 report@12846     | document binary pdf    | 1.2MB | mod 3d
4 @12847           | document utf8          | 4.2KB | mod 6h

# Show only canonical node IDs (name becomes an attribute)
alice (lobbox1) >> qr -a type:document -oid
12844 | draft document utf8 markdown      | 2.4KB | mod 2h
12845 | notes document utf8 markdown      | 8.1KB | mod 1d
12846 | report document binary pdf        | 1.2MB | mod 3d
12847 | document utf8                     | 4.2KB | mod 6h

# Find large images created this week
alice (lobbox1) >> qr -a type:image -s '>1MB' -c 7d
1 screenshot | image binary png  | 2.3MB | cr 2d
2 photo      | image binary jpeg | 4.1MB | cr 5d

# With canonical IDs
alice (lobbox1) >> qr -a type:image -s '>1MB' -c 7d -sid
1 screenshot@9284 | image binary png  | 2.3MB | cr 2d
2 photo@9285      | image binary jpeg | 4.1MB | cr 5d

# Find everything firefox created
alice (lobbox1) >> qr -b @8472
1 profile  | database binary sqlite | 45KB  | cr 30d
2 cache    | database binary sqlite | 2.1MB | cr 30d
3 bookmark | data utf8 json         | 12KB  | cr 15d

# Find nodes owned by a package
alice (lobbox1) >> qr -o @1234
1 firefox-bin   | executable binary elf | 89MB  | 
2 crashreporter | executable binary elf | 2.3MB |
3 icons         | image binary png      | 128KB |

# Find nodes currently in use (refcount > 0)
alice (lobbox1) >> qr -iu
1 firefox-bin | executable binary elf | 89MB | rc:3
2 vim-bin     | executable binary elf | 12MB | rc:1

# With canonical IDs
alice (lobbox1) >> qr -iu -sid
1 firefox-bin@1192 | executable binary elf | 89MB | rc:3
2 vim-bin@8291     | executable binary elf | 12MB | rc:1

# Only canonical IDs
alice (lobbox1) >> qr -a data:binary -oid
1192 | firefox executable binary elf  | 89MB  | rc:3 | cr 12m
1009 | screenshot image binary png    | 2.3MB | rc:0 | cr 2d
8291 | vim executable binary elf      | 12MB  | rc:1 | cr 45d

# Combine multiple filters
alice (lobbox1) >> qr -a type:document,tag:work -m 7d -s '<100KB'
1 todo  | document utf8 markdown | 1.2KB | mod 2d
2 notes | document utf8 markdown | 8.4KB | mod 5d
```

---

Here's the section:

---

## Name Resolution

The `@` prefix references a node directly. It can be used with a numeric ID or a name:

| Syntax | Meaning |
|--------|---------|
| `@12844` | exact node ID reference |
| `@firefox` | resolve by `name` attribute |
| `@firefox:package` | resolve by `name` and `type` attributes |

Name resolution is unambiguous when exactly one node matches. If multiple nodes share a name, the shell presents an inline disambiguation prompt using the standard numbered list format:

```shell
alice (lobbox1) >> drop @firefox
Error: @firefox is ambiguous - 3 nodes match name:firefox
1 @8291 firefox-bin | executable
2 @7819 firefox     | package
3 @9285 firefox     | process
Which would you like to drop? >> 2
Will drop owned nodes: 9285, 8291, 11299, 880, 881, 12850, 12840
Drop node 7819 (firefox)? [y/N] y
Dropped nodes: 7819, 9285, 8291, 11299, 880, 881, 12850, 12840
```

The type hint syntax narrows resolution before prompting:

```shell
# unambiguous - only one package named firefox
alice (lobbox1) >> drop @firefox:package
Will drop owned nodes: 9285, 8291, 11299, 880, 881, 12850, 12840
Drop node 7819 (firefox)? [y/N] y
Dropped nodes: 7819, 9285, 8291, 11299, 880, 881, 12850, 12840

# still ambiguous - two processes named vim
alice (lobbox1) >> show @vim:process
Error: @vim:process is ambiguous - 2 nodes match name:vim, type:process
1 @9284 vim  | process | cr 2h
2 @9291 vim  | process | cr 4h
Which would you like to show? >> 1
```

The disambiguation prompt sets the context node, so `lqr` operates on the resolved result afterward. Any command that accepts a `@name` reference uses this same resolution behavior - it is not specific to any individual command.

---

## Query Chaining and Graph Traversal

### `lqr` - Local Query

`lqr` is identical to `qr` but operates only on the current context - the result set of the last query. Returns an error if no context exists.

```shell
alice (lobbox1) >> lqr -a type:document
Error: No context - run a query first

# find all documents in alice's nodes
alice (lobbox1) >> qr -u alice
1 draft  | document utf8 markdown | 2.4KB
2 notes  | document utf8 markdown | 8.1KB
3 vim    | process                 | cr 2h
4 config | config utf8 toml       | 892B

alice (lobbox1) >> lqr -a type:document
1 draft | document utf8 markdown | 2.4KB
2 notes | document utf8 markdown | 8.1KB
```

### Traversal Operators

Traversal operators walk edges from the current result set. They use the same edge abbreviations as `qr` flags: `o` (own), `r` (ref), `w` (weak).

| Operator | Meaning |
|----------|---------|
| `.o` | replace result set with nodes owned by current set (one hop) |
| `.r` | replace result set with nodes referenced by current set (one hop) |
| `.w` | replace result set with nodes weakly referenced by current set (one hop) |
| `.o+` | replace result set with full ownership subtree (one or more hops) |
| `.o*` | replace result set with full ownership subtree including current set |

Set operators combine the current result set with the traversal result:

| Operator | Meaning |
|----------|---------|
| `\|.o` | union - add owned children to current set |
| `&.o` | intersection - keep only nodes that own something |
| `-.o` | difference - remove nodes that own something |

Traversal operators are appended to any command that produces a result set:

```shell
# nodes owned by alice's nodes
alice (lobbox1) >> qr -u alice .o

# alice's nodes plus their entire ownership subtree
alice (lobbox1) >> qr -u alice |.o+

# only alice's nodes that own something
alice (lobbox1) >> qr -u alice &.o

# all documents anywhere in firefox's ownership tree
alice (lobbox1) >> qr -b @firefox |.o+
alice (lobbox1) >> lqr -a type:document

# find all nodes that reference something in alice's tree
alice (lobbox1) >> qr -u alice |.o+
alice (lobbox1) >> lqr &.r

# walk two ownership levels then find anything with an active ref
alice (lobbox1) >> qr -u alice .o .o &.r

# find nodes created by bob anywhere in alice's ownership tree
alice (lobbox1) >> qr -u alice |.o+
alice (lobbox1) >> lqr -u bob
```

### Inline Chaining with `|`

Multiple `lqr` operations can be chained on one line using `|`. The `|` operator passes the current result set to the next set of flags, implicitly scoping them as `lqr`:

```shell
# equivalent to the two-line version above
alice (lobbox1) >> qr -u alice |.o+ | -u bob

# find all documents modified this week in firefox's ownership tree
alice (lobbox1) >> qr -b @firefox |.o+ | -a type:document | -m 7d

# find all nodes owned by running processes except documents
alice (lobbox1) >> qr -a type:process &.o | -.a type:document

# find everything bob created inside alice's subtree, then walk their weak refs
alice (lobbox1) >> qr -u alice |.o+ | -u bob .w+

# find orphaned nodes - weakly referenced but not in any active ownership tree
alice (lobbox1) >> qr -iu |.o+
alice (lobbox1) >> lqr &.w -.o
```

---

## Syscall Commands

The shell exposes most of the LOBNS syscall API directly as commands:

| Command | Syscall | Description |
|---------|---------|-------------|
| `move` | `move(node, to)` | Transfer ownership |
| `clone` | `clone(node, to)` | Duplicate node with new ID and owner |
| `ref` | `ref(node)` | Create Ref edge (borrow) |
| `drop` | `drop(node)` | Release ownership, trigger cascade |
| `weak` | `weak(node)` | Create Weak edge |
| `upgrade` | `upgrade(weak_ref)` | Promote Weak to Ref |

---

## Common Aliases

### `mine`

Alias for `qr -u self` - shows nodes you created:

```shell
alice (lobbox1) >> mine
1 draft         | document utf8 markdown | 2.4KB
2 notes         | document utf8 markdown | 8.1KB
3 shell-config  | config utf8 toml       | 1.2KB

alice (lobbox1) >> mine -sid
1 draft@12844         | document utf8 markdown | 2.4KB
2 notes@12845         | document utf8 markdown | 8.1KB
3 shell-config@12846  | config utf8 toml       | 1.2KB
```

### `recent`

Alias for `qr -c 1d` - shows nodes created in the last 24 hours:

```shell
alice (lobbox1) >> recent
1 screenshot | image binary png       | 2.3MB | 4h ago
2 draft      | document utf8 markdown | 1.2KB | 8h ago
3 download   | archive binary zip     | 45MB  | 12h ago
```

### `modified`

Alias for `qr -m 1d` - shows nodes modified in the last 24 hours:

```shell
alice (lobbox1) >> modified
1 notes  | document utf8 markdown | 8.1KB | 2h ago
2 config | config utf8 toml       | 892B  | 6h ago
```

### `procs`

Alias for `qr -a type:process`

```shell
# Show all running processes
alice (lobbox1) >> procs
1 vim            | process | rc:1 | started 2h ago
2 firefox        | process | rc:3 | started 4h ago
3 shell          | process | rc:0 | started 6h ago

# With canonical IDs
alice (lobbox1) >> procs -sid
1 vim@9284       | process | rc:1 | started 2h ago
2 firefox@9285   | process | rc:3 | started 4h ago
3 shell@9286     | process | rc:0 | started 6h ago

# Terminate a process (just drop)
alice (lobbox1) >> drop 2
Will drop owned nodes from 9285: 11203, 11204, 11205
Drop node 9285 (firefox)? [y/N] y
Dropped nodes: 9285, 11203, 11204, 11205

# Find processes created by a binary
alice (lobbox1) >> procs | qr -b @8291
1 vim@9284 | process | rc:1 | started 2h ago

# Find all nodes owned by a running process
alice (lobbox1) >> qr -o @9285
1 tab-1   | document utf8 html | 124KB | cr 4h
2 tab-2   | document utf8 html | 89KB  | cr 3h
3 cache   | database binary    | 2.1MB | cr 4h
```

---

## Working with Query Results

Query results are numbered and can be referenced in subsequent commands.

### `show` - Display Node Details

```shell
alice (lobbox1) >> mine
1 draft  | document utf8 markdown | 2.4KB
2 notes  | document utf8 markdown | 8.1KB

alice (lobbox1) >> show 1
Node ID: 12844
Owner: @9281 (alice-session)
Refcount: 0
Content Hash: blake3:a7f8e9d2c1b4...
Created: 2024-03-11 14:23:11 by vim (@8291)
Modified: 2024-03-11 16:45:32
Accessed: 2024-03-11 16:50:01

Attributes:
  type: document
  data: utf8
  format: markdown
  name: draft
  tag: work

Edges:
  (none)
```

### `show @id` - Direct Node Reference

```shell
alice (lobbox1) >> show @12844
Node ID: 12844
Owner: @9281 (alice-session)
...
```

### `dump` - Display Node Data (Read-Only)

```shell
alice (lobbox1) >> dump 1
Hello, world!
This is a draft document.

alice (lobbox1) >> dump @12844
Hello, world!
This is a draft document.

# This is a convenience wrapper around ref() for read-only access
# Internally creates temporary Ref edge, reads data, drops Ref
```

### `edit` - Edit Node with Exclusive Lease

```shell
alice (lobbox1) >> edit 1
# Opens $EDITOR with node data
# Acquires exclusive write lease (ref_mut)
# On save, writes back and releases lease

alice (lobbox1) >> edit @12844
# Same, but direct node reference

# Error if another ref_mut is active
alice (lobbox1) >> edit 1
Error: Node 12847 has an active write lease: @1180
```

### `drop` - Release Ownership

Drop releases ownership of a node, triggering cascade deletion of all owned nodes.

```shell
# Drop shows what will be cascaded
alice (lobbox1) >> drop 1
Will drop owned nodes: 11299, 880, 881, 12850, 12840
Drop node 12844 (bad-process)? [y/N] y
Dropped nodes: 12844, 11299, 880, 881, 12850, 12840

# If no owned nodes, simpler message
alice (lobbox1) >> drop 2
Drop node 12845 (notes)? [y/N] y
Dropped node 12845

# Force without confirmation
alice (lobbox1) >> drop -f 2
Dropped node 12845

# Drop multiple (shows cascade for each)
alice (lobbox1) >> drop 1 2 3
Will drop owned nodes from 12844: 11299, 880, 881
Will drop owned nodes from 12845: (none)
Will drop owned nodes from 12846: 9281, 9282
Drop 3 nodes (total 6 with cascade)? [y/N] y
Dropped nodes: 12844, 11299, 880, 881, 12845, 12846, 9281, 9282

# Can also use canonical node IDs
alice (lobbox1) >> drop @12844
Will drop owned nodes: 11299, 880, 881, 12850, 12840
Drop node 12844 (bad-process)? [y/N] y
Dropped nodes: 12844, 11299, 880, 881, 12850, 12840

# Error if refcount > 0
alice (lobbox1) >> drop 1
Error: Cannot drop node 12844 - refcount is 2
Referenced by: 9281, 12903
```

---

## Node Creation

### `new` - Create New Node

```shell
# creates node with data:utf8 by default
alice (lobbox1) >> new -a name:todo
Created node 12849
```

### `clone` - Duplicate Node (Memory Management Semantics)

Clone creates a duplicate node with a new ID and assigns ownership. This matches Rust's `clone()` semantics - duplicate the data and assign it to a new owner.

```shell
# Clone node 1, unowned (persistent)
alice (lobbox1) >> clone 1 unowned
Created node 12851 (clone of 12844, unowned)

# Clone node 1, new node owned by another node
alice (lobbox1) >> clone 1 @9281
Created node 12851 (clone of 12844, owned by @9281)

# Clone using result number and node ID
alice (lobbox1) >> clone 4 2
Created node 12852 (clone of result 4, owned by result 2)

# Clone with direct node IDs
alice (lobbox1) >> clone @1392 @1235
Created node 12853 (clone of @1392, owned by @1235)

# Clone with single argument - same owner as original
alice (lobbox1) >> clone 1
Created node 12854 (clone of 12844, owned by @9281)
```

---

## Edge Operations

### `edges` - Show Node Edges

```shell
alice (lobbox1) >> edges 1
Node 12844 (draft)

Outgoing edges:
  Own  → @12901 (revision-1)
  Own  → @12902 (revision-2)
  Weak → @8291 (vim-bin)

Incoming edges:
  Ref  ← @9281 (alice-session)
  Weak ← @12903 (backup)

# Show only outgoing edges
alice (lobbox1) >> edges 1 -d out
Outgoing edges:
  Own  → @12901 (revision-1)
  Own  → @12902 (revision-2)
  Weak → @8291 (vim-bin)

# Show only incoming edges
alice (lobbox1) >> edges 1 -d in
Incoming edges:
  Ref  ← @9281 (alice-session)
  Weak ← @12903 (backup)

# Show only Own edges
alice (lobbox1) >> edges 1 -o
Outgoing Own edges:
  Own  → @12901 (revision-1)
  Own  → @12902 (revision-2)

# Show only Ref edges
alice (lobbox1) >> edges 1 -r
Incoming Ref edges:
  Ref  ← @9281 (alice-session)

# Show only Weak edges
alice (lobbox1) >> edges 1 -w
Outgoing Weak edges:
  Weak → @8291 (vim-bin)

Incoming Weak edges:
  Weak ← @12903 (backup)

# Combine filters: only outgoing Weak edges
alice (lobbox1) >> edges 1 -d out -w
Outgoing Weak edges:
  Weak → @8291 (vim-bin)
```

### `move` - Transfer Ownership

```shell
# Move to unowned (persist)
alice (lobbox1) >> move 1 unowned
Moved node 12844 to unowned (now persistent and journaled)

# Move to another node
alice (lobbox1) >> move 1 @9281
Moved node 12844 to owner @9281

# Error if refs exist
alice (lobbox1) >> move 1 unowned
Error: Cannot move node 12844 - refcount is 2
Active refs from: @9281, @12903

# Error if ref_mut is active
alice (lobbox1) >> move 1 unowned
Error: Cannot move node 12844 - exclusive write lease is active
```

### `ref` - Create Reference Edge (Borrow)

```shell
# Create Ref edge from node 1 to node 2
alice (lobbox1) >> ref 1 2
Created Ref edge: 12844 → 12848
Node 12848 refcount: 0 → 1

# Direct node IDs
alice (lobbox1) >> ref @12844 @12848
Created Ref edge: 12844 → 12848

# Ref edges prevent deletion
alice (lobbox1) >> drop 2
Error: Cannot drop node 12848 - refcount is 1
Referenced by: @12844
```

### `weak` - Create Weak Edge

```shell
# Create Weak edge from node 1 to node 2
alice (lobbox1) >> weak 1 2
Created Weak edge: 12844 → 12848

# Weak edges can have attributes
alice (lobbox1) >> weak 1 2 -a label:"previous version"
Created Weak edge: 12844 → 12848 (label: previous version)

# Weak edges don't prevent deletion
alice (lobbox1) >> drop 2
Dropped node 12848
# Node 1's weak edge becomes tombstone
```

### `upgrade` - Promote Weak to Ref

```shell
# Attempt to upgrade weak edge to ref
alice (lobbox1) >> upgrade 1 2
Upgraded Weak edge to Ref: 12844 → 12848
Node 12848 refcount: 0 → 1

# Fails if target is tombstone
alice (lobbox1) >> upgrade 1 2
Error: Cannot upgrade - node 12848 is tombstone (target was dropped)
```

### `unlink` - Remove Edge

```shell
alice (lobbox1) >> unlink 1 2
Remove edge 12844 → 12848? [y/N] y
Removed edge

# Specify edge type
alice (lobbox1) >> unlink -t ref 1 2
Removed Ref edge: 12844 → 12848
Node 12848 refcount: 1 → 0
```

---

## Attribute Operations

### `attr` - Show/Set Attributes

The `attr` command displays all attributes as a key: value list, or sets/removes attributes.

```shell
# Show all attributes (key: value list)
alice (lobbox1) >> attr 1
type: document
data: utf8
format: markdown
name: draft
tag: work

# Show specific attribute
alice (lobbox1) >> attr 1 name
draft

# Set attribute
alice (lobbox1) >> attr 1 tag:urgent
Set tag=urgent on node 12844

# Set multiple
alice (lobbox1) >> attr 1 tag:urgent,priority:high
Set tag=urgent, priority=high on node 12844

# Remove attribute
alice (lobbox1) >> attr 1 tag:
Removed tag from node 12844
```

### `tag` - Shorthand for Tag Attributes

```shell
# Add tags
alice (lobbox1) >> tag work 1 2 3
Tagged nodes 12844, 12848, 12849 with 'work'

# Remove tags
alice (lobbox1) >> untag work 1 2
Removed tag 'work' from nodes 12844, 12848

# Query by tag
alice (lobbox1) >> qr -a tag:work
1 draft | document utf8 markdown | 2.4KB
2 notes | document utf8 markdown | 8.1KB
3 todo  | document utf8 markdown | 1.2KB
```

---

## Provenance and History

### `trace` - Show Creation Chain

```shell
alice (lobbox1) >> trace 1
Node 12844 (draft)
  created_by_process: @9284 (vim instance) [tombstone]
  created_by_binary: @8291 (vim-bin)
  created_by_user: alice
  created_at: 2024-03-11 14:23:11

Binary @8291 (vim-bin)
  created_by_process: @7821 (package-manager) [tombstone]
  created_by_binary: @7820 (pkg-install)
  created_by_user: root
  created_at: 2024-02-15 09:12:43

Package @7819 (vim-package)
  created_by_binary: @7820 (pkg-install)
  created_by_user: root
  created_at: 2024-02-15 09:12:40
```

---

## Piping and Composition

```shell
# Tag all images from last week
alice (lobbox1) >> qr -a type:image -c 7d | tag recent

# Delete all old temp files
alice (lobbox1) >> qr -a type:temp -c '<30d' | drop -f

# Show details of all in-use binaries
alice (lobbox1) >> qr -a type:executable -iu | show

# Archive old documents
alice (lobbox1) >> qr -a type:document -m '<90d' | attr archived:true
```

---

## Package Management

### `plob` - Package Operations

```shell
# List installed packages
alice (lobbox1) >> plob list
1 firefox        | 89.2MB  | installed 30d ago
2 vim            | 12.4MB  | installed 45d ago
3 rust-toolchain | 234MB   | installed 60d ago

# Show package contents
alice (lobbox1) >> plob show firefox
Package: firefox (@7819)
Size: 89.2MB
Installed: 2024-02-15 09:12:40

Owned nodes:
  @8291 firefox-bin (executable, 87MB)
  @8292 crashreporter (executable, 2.1MB)
  @8293 icons.png (image, 128KB)

# Install package
alice (lobbox1) >> plob install rust-toolchain
# Downloads manifest, creates package node, installs binaries

# Uninstall package
alice (lobbox1) >> plob remove firefox
Remove package firefox and all owned nodes? [y/N] y
# Single drop_node() call - cascade handles everything
```

---

## System Queries

### `nsstats` - LOBNS Stats

```shell
alice (lobbox1) >> nsstats
LOB Node Store Statistics:
  Total nodes: 48,291
  Unowned nodes: 12,847
  Ephemeral nodes: 1,284
  Journaled nodes: 46,007
  
  Total edges: 89,472
    Own: 12,483
    Ref: 8,291
    Weak: 68,698

  Storage: 12.4GB used, 487GB free
  Memory: 2.1GB node cache, 4.8GB anonymous

alice (lobbox1) >> nsstats -u alice
Alice's nodes: 1,847
  Unowned: 892
  Owned by session: 955
  Total size: 2.3GB
```

### `sysinfo` - neofetch style system information

```shell
alice (lobbox1) >> sysinfo
alice@lob
-----------
OS: LOB x86_64
Host: Latitude E7450
Kernel: lob-kernel 2.4-amd64
Uptime: 6 hours, 31 mins
Packages: 2255 (plob)
Shell: shell 1.9
Display (BOE05F3): 1366x768 in 14 in, 60 Hz [Built]
DE: lob-desktop 1.20
Terminal: lob-terminal 1.1.5
Terminal Font: Less Perfect DOS VGA (12pt)
CPU: Intel(R) Core(TM) i7-5600U (4) @ 3.20 GHz
GPU: Intel HD Graphics 5500 @ 0.95 GHz [Integrat]
Memory: 5.88 GiB / 15.49 GiB (38%)
Swap: 0 B / 15.87 GiB (0%)
Disk (/): 81.77 GiB / 440.82 GiB (19%) - ext4
Local IP (wlp2s0): 192.168.0.102/24
Battery (DELL 909H538): 38% (59 mins remaining) ]
Locale: en_US.UTF-8
```
---

## Scripting

The shell can execute scripts with full access to all commands:

```shell
#!shell

# backup-documents
# Backs up all documents to archive node

qr -a type:document -m 7d | while read node; do
    clone $node -o @archive-node -a archived:true
done

echo "Backup complete"
```

Run with:
```shell
alice (lobbox1) >> exec backup-documents
```

---

## Configuration

Shell configuration is itself a node:

```shell
alice (lobbox1) >> edit @shell-config

# shell configuration
editor = "vim"
pager = "less"
confirm_delete = true
date_format = "relative"  # or "absolute"
color = true

# Aliases
alias docs = "qr -a type:document"
alias imgs = "qr -a type:image"
alias work = "qr -a tag:work"
```

