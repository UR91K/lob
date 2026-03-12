# lob-shell Quick Reference

> **See also:** [Concepts](./lob-shell-concepts.md) - ownership, edge types, node lifecycle  
> **See also:** [Cookbook](./lob-shell-cookbook.md) - piping, scripting, real-world workflows

---

## Syntax Conventions

| Syntax | Meaning |
|--------|---------|
| `1`, `2`, `3` | Result number from last query |
| `@12844` | Exact node ID |
| `@firefox` | Resolve by `name` attribute |
| `@firefox:package` | Resolve by `name` and `type` |
| `unowned` | Special target - no owner (persistent) |

If a name resolves to multiple nodes, the shell prompts inline disambiguation. Type hints (`:package`, `:process`) narrow resolution before prompting.

---

## `qr` / `query` - Query the Node Store

```
qr [flags] [traversal]
```

| Flag | Short | Argument | Description |
|------|-------|----------|-------------|
| `--id` | `-i` | `<node-id>` | Exact node ID |
| `--owner-id` | `-o` | `@<id/name>` | Owned by this node |
| `--reference-from-id` | `-r` | `@<id/name>` | Has Ref edge from this node |
| `--weak-from-id` | `-w` | `@<id/name>` | Has Weak edge from this node |
| `--size` | `-s` | `<size-spec>` | `1KB`, `1-4KB`, `>8MB`, `<14GB` |
| `--reference-count` | `-rc` | `<count>` | Exactly this refcount |
| `--in-use` | `-iu` | *(flag)* | Refcount > 0 |
| `--created` | `-c` | `<date/range>` | Creation timestamp |
| `--modified` | `-m` | `<date/range>` | Modification timestamp |
| `--accessed` | `-ac` | `<date/range>` | Access timestamp |
| `--process` | `-p` | `@<id/name>` | Created by this process instance |
| `--binary` | `-b` | `@<id/name>` | Created by this binary |
| `--user` | `-u` | `<username>` | Created by this user |
| `--attributes` | `-a` | `key:value,...` | Match attributes |
| `--show-id` | `-sid` | *(flag)* | Show `name@id` alongside names |
| `--only-id` | `-oid` | *(flag)* | Show only node IDs (name becomes attribute) |

### Date/Range Syntax

| Syntax | Meaning |
|--------|---------|
| `2024-03-11` | Exact date |
| `2024-03-11T14:30:00` | Exact timestamp |
| `>2024-03-01` | After this date |
| `<2024-03-01` | Before this date |
| `2024-03-01..2024-03-11` | Range (inclusive) |
| `1d` | Last 24 hours |
| `7d` | Last 7 days |
| `30d` | Last 30 days |

---

## `lqr` - Local Query (Current Context)

Identical to `qr` but scoped to the current result set. Errors if no context exists.

```
lqr [flags] [traversal]
```

Multiple `lqr` operations can be chained inline with `|`:

```shell
qr -u alice |.o+ | -a type:document | -m 7d
```

---

## Traversal Operators

Appended to any command that produces a result set.

### Replace Operators

| Operator | Meaning |
|----------|---------|
| `.o` | Nodes owned by current set (one hop) |
| `.r` | Nodes referenced by current set (one hop) |
| `.w` | Nodes weakly referenced by current set (one hop) |
| `.o+` | Full ownership subtree (one or more hops) |
| `.o*` | Full ownership subtree including current set |

### Set Operators

| Operator | Meaning |
|----------|---------|
| `\|.o` | Union - add owned children to current set |
| `&.o` | Intersection - keep only nodes that own something |
| `-.o` | Difference - remove nodes that own something |

---

## Aliases

| Alias | Expands to | Description |
|-------|-----------|-------------|
| `mine` | `qr -u self` | Nodes you created |
| `recent` | `qr -c 1d` | Created in last 24 hours |
| `modified` | `qr -m 1d` | Modified in last 24 hours |
| `procs` | `qr -a type:process` | All processes |

All aliases accept the same flags as `qr` (e.g. `mine -sid`, `procs -iu`).

---

## Node Commands

### `show` - Display Node Details

```
show <ref>
```

Shows ID, owner, refcount, content hash, timestamps, all attributes, and edges.

### `dump` - Display Node Data

```
dump <ref>
```

Read-only. Internally creates a temporary Ref, reads data, drops Ref.

### `edit` - Edit Node with Exclusive Lease

```
edit <ref>
```

Opens `$EDITOR`. Acquires exclusive write lease (`ref_mut`). Errors if another write lease is active.

### `new` - Create Node

```
new [-a key:value,...]
```

Creates a node with `data:utf8` by default.

```shell
new -a name:todo
```

### `clone` - Duplicate Node

```
clone <source> [<owner>]
```

Duplicates data, assigns new ID. If owner omitted, inherits original owner.

```shell
clone 1              # same owner as original
clone 1 unowned      # no owner (persistent)
clone 1 @9281        # owned by @9281
clone 4 2            # result 4, owned by result 2
```

### `drop` - Release Ownership

```
drop [-f] <ref> [<ref>...]
```

Triggers cascade deletion of all owned nodes. Shows preview before confirming.

```shell
drop 1               # with confirmation
drop -f 1            # force, no confirmation
drop 1 2 3           # multiple, one confirmation
```

Errors if refcount > 0.

### `move`, `mv` - Transfer Ownership

```
mv <node> <new-owner>
```

```shell
move 1 unowned       # make persistent
move 1 @9281         # transfer to another node
```

Errors if refcount > 0 or a write lease is active.

This has noticably different behaviour from the `mv` command in a Unix environment

---

## Edge Commands

### `edges` - Show Edges

```
edges <ref> [-d in|out] [-o] [-r] [-w]
```

| Flag | Description |
|------|-------------|
| `-d in` / `-d out` | Filter by direction |
| `-o` | Own edges only |
| `-r` | Ref edges only |
| `-w` | Weak edges only |

### `ref` - Create Ref Edge (Borrow)

```
ref <from> <to>
```

Increments refcount on target. Target cannot be dropped while refcount > 0.

### `weak` - Create Weak Edge

```
weak <from> <to> [-a key:value,...]
```

Does not affect refcount. If target is dropped, edge becomes a tombstone.

### `upgrade`, `up` - Promote Weak to Ref

```
up <from> <to>
```

Fails if target is a tombstone.

### `unlink`, `ul` - Remove Edge

```
ul [-t ref|weak|own] <from> <to>
```

---

## Attribute Commands

### `attr` - Show or Set Attributes

```
attr <ref> [key[:value],...]
```

```shell
attr 1                        # show all attributes
attr 1 name                   # show one attribute
attr 1 tag:urgent             # set attribute
attr 1 tag:urgent,priority:1  # set multiple
attr 1 tag:                   # remove attribute
```

### `tag` / `untag` - Shorthand Tag Operations

```
tag <tag-name> <ref> [<ref>...]
untag <tag-name> <ref> [<ref>...]
```

---

## Provenance

### `trace` - Show Creation Chain

```
trace <ref>
```

Walks the provenance chain: node → process → binary → package, showing creator, user, and timestamp at each level. Tombstoned entries are marked.

---

## Package Management (`plob`)

| Command | Description |
|---------|-------------|
| `plob list` | List installed packages |
| `plob show <name>` | Show package contents and owned nodes |
| `plob install <name>` | Install package |
| `plob remove <name>` | Uninstall package and all owned nodes |

---

## System Commands

| Command | Description |
|---------|-------------|
| `nsstats` | Node store statistics (total nodes, edges, storage) |
| `nsstats -u <user>` | Statistics scoped to a user |
| `sysinfo` | System info (neofetch-style) |
| `exec <script>` | Run a lob-shell script |

---

## Configuration

The shell config is itself a node, editable with:

```shell
edit @lob-shell-cfg
```

| Key | Default | Description |
|-----|---------|-------------|
| `editor` | `"vim"` | Editor for `edit` command |
| `pager` | `"less"` | Pager for long output |
| `confirm_delete` | `true` | Prompt before `drop` |
| `date_format` | `"relative"` | `"relative"` or `"absolute"` |
| `color` | `true` | Colored output |

Custom aliases can be defined in config:

```toml
alias docs = "qr -a type:document"
alias work = "qr -a tag:work"
```