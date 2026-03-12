# lob-shell Cookbook

> **See also:** [Quick Reference](./reference.md) — all commands and flags  
> **See also:** [Concepts](./concepts.md) — ownership, edge types, node lifecycle

---

## Querying and Filtering

### Find nodes by type

```shell
qr -a type:document
qr -a type:image
qr -a type:process
qr -a type:executable
```

### Combine multiple attribute filters

```shell
# Documents tagged 'work', modified this week, under 100KB
qr -a type:document,tag:work -m 7d -s <100KB
```

### Find everything a user created

```shell
qr -u alice
qr -u self        # your own nodes (same as: mine)
```

### Find everything a binary created

```shell
qr -b @firefox
qr -b @vim
```

### Find large files

```shell
qr -s >100MB
qr -a type:image -s >1MB
qr -s 10MB-1GB
```

### Find recently created or modified nodes

```shell
recent                    # created in last 24h (alias)
modified                  # modified in last 24h (alias)
qr -c 7d                  # created in last 7 days
qr -m 30d                 # modified in last 30 days
qr -c 2024-03-01..2024-03-11
```

### Find nodes currently in use

```shell
qr -iu                    # refcount > 0
qr -a type:executable -iu # only in-use executables
```

### Show canonical node IDs in results

```shell
mine -sid                 # name@id format
qr -a type:process -oid   # id only, name becomes attribute
```

---

## Graph Traversal

### Explore an ownership tree

```shell
# Direct children of alice's nodes
qr -u alice .o

# Everything alice owns, recursively
qr -u alice .o+

# Everything alice owns, including her nodes themselves
qr -u alice .o*
```

### Find nodes inside a package or process

```shell
# Everything firefox owns
qr -o @firefox:package

# All documents anywhere in firefox's ownership tree
qr -b @firefox |.o+
lqr -a type:document
```

### Find nodes created by one user inside another's tree

```shell
qr -u alice |.o+
lqr -u bob
```

Or inline:

```shell
qr -u alice |.o+ | -u bob
```

### Find active borrows inside a subtree

```shell
# Nodes in alice's tree that have active Ref edges pointing to them
qr -u alice |.o+ &.r
```

### Walk two ownership levels then filter

```shell
qr -u alice .o .o -a type:document
```

### Find weak references inside a tree

```shell
qr -u alice |.o+ | -m 7d .w
```

---

## Working with Results

### Inspect a node

```shell
mine
show 1            # full detail on result 1
dump 1            # print raw data
attr 1            # list all attributes
edges 1           # show all edges
trace 1           # show provenance chain
```

### Edit a node

```shell
edit 1            # opens $EDITOR, acquires write lease
edit @12844       # same, by direct ID
```

### Tag results in bulk

```shell
qr -a type:image -c 7d | tag recent
qr -a type:document,tag:work | tag priority
```

### Set attributes on results in bulk

```shell
qr -a type:document -m '<90d' | attr archived:true
qr -a tag:temp | attr status:stale
```

---

## Ownership and Persistence

### Persist a node beyond its owner's lifetime

```shell
move 1 unowned
```

### Clone a node and persist the copy

```shell
clone 1 unowned
```

### Transfer a node to another owner

```shell
move 1 @9281
move 1 @archive-node
```

### Clone into a specific owner

```shell
clone 1 @archive-node
```

### Check what owns a node

```shell
show 1            # Owner field in output
```

### Check what a node owns

```shell
edges 1 -o        # outgoing Own edges only
qr -o @12844      # query owned nodes directly
```

---

## Processes

### List running processes

```shell
procs
procs -sid        # with canonical IDs
procs -iu         # only processes with active refs
```

### Find the process that created a node

```shell
trace 1           # shows created_by_process
```

### Find all nodes owned by a process

```shell
qr -o @9285
```

### Find all nodes a process created (across its lifetime)

```shell
qr -p @9285
```

### Terminate a process

```shell
procs
drop 2            # cascade drops everything it owns
drop -f 2         # no confirmation
```

### Find processes created by a specific binary

```shell
procs | -b @8291
```

---

## Packages

### List, inspect, install, remove

```shell
plob list
plob show firefox
plob install rust-toolchain
plob remove firefox
```

### Find all nodes a package owns

```shell
qr -o @firefox:package
```

### Find all nodes anywhere in a package's tree

```shell
qr -o @firefox:package |.o+
```

### Check what binaries a package installed

```shell
qr -o @firefox:package -a type:executable
```

---

## Cleanup and Maintenance

### Drop all nodes matching a query (with confirmation)

```shell
qr -a type:temp | drop
```

### Force-drop without confirmation

```shell
qr -a type:temp -m '<30d' | drop -f
```

### Find orphaned weak references (tombstones)

After a target is dropped, its incoming weak edges become tombstones. To find nodes with outgoing weak edges that point to tombstones:

```shell
# Find nodes with weak edges, then check which targets are gone
qr -a tag:cache .w
# Any tombstoned targets will be marked in the edges output
```

### Find large nodes you own

```shell
mine -s '>10MB'
```

### Archive old documents

```shell
qr -a type:document -m '<90d' | attr archived:true
qr -a type:document,archived:true | move unowned
```

---

## Borrowing and References

### Borrow a node (create Ref edge)

```shell
ref 1 2           # node 1 borrows node 2
```

### Check what's borrowing a node

```shell
edges 2 -r        # incoming Ref edges on node 2
```

### Release a borrow

```shell
unlink -t ref 1 2
```

### Upgrade a weak reference to a borrow

```shell
upgrade 1 2       # fails if node 2 is a tombstone
```

---

## Scripting

Scripts use the same commands as the interactive shell. Run them with `exec`.

### Backup recent documents to an archive node

```shell
#!lob-shell

# backup-recent-docs
# Clones all documents modified in the last 7 days into @archive-node

qr -a type:document -m 7d | while read node; do
    clone $node @archive-node -a archived:true
done

echo "Backup complete"
```

### Clean up stale temp nodes

```shell
#!lob-shell

# cleanup-temps
# Drops all temp nodes older than 30 days

qr -a type:temp -c '<30d' | drop -f
echo "Cleanup complete"
```

### Report disk usage by type

```shell
#!lob-shell

# usage-by-type
# Prints node count and total size for each type

for type in document image executable database archive; do
    qr -a type:$type | nsstats
done
```

Run any script with:

```shell
exec backup-recent-docs
exec cleanup-temps
```

---

## Configuration and Aliases

### Edit shell config

```shell
edit @lob-shell-cfg
```

### Define custom aliases

```toml
alias docs   = "qr -a type:document"
alias imgs   = "qr -a type:image"
alias work   = "qr -a tag:work"
alias big    = "qr -s '>100MB'"
alias active = "qr -iu"
```

Once defined, aliases accept the same flags as `qr`:

```shell
docs -m 7d -sid
work -s '<10KB'
```