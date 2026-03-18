<!--
SYNC IMPACT REPORT
==================
Version Change:        N/A → 1.0.0 (initial ratification)
Modified Principles:   N/A — first ratification
Added Sections:        Core Principles (I–VII), Technical Standards,
                       Development Phases, Governance
Removed Sections:      N/A
Templates Reviewed:
  - .specify/templates/plan-template.md   ✅ Constitution Check gates are generic; no update needed
  - .specify/templates/spec-template.md   ✅ No structural changes required
  - .specify/templates/tasks-template.md  ✅ No structural changes required
  - .specify/templates/checklist-template.md  ✅ Not reviewed; no known dependency
Deferred TODOs:        None
-->

# LOB Operating System Constitution

## Core Principles

### I. Invariant Enforcement (NON-NEGOTIABLE)

The eight graph invariants MUST be enforced at every syscall boundary with no
exceptions:

1. Every node has exactly one owner, or is explicitly unowned.
2. A node MUST NOT be moved while any Ref edge points to it.
3. A node MUST NOT be moved while any ref_mut lease is active on it.
4. At most one ref_mut lease may exist for a node at any time.
5. Ownership edges MUST form a DAG; cycles are rejected at edge creation.
6. Dropping an owner MUST cascade deterministically to all owned nodes.
7. Weak edges MUST NOT prevent deletion; `upgrade()` MAY return `None`.
8. Node data MUST NOT be mutated without an active ref_mut lease.

An invalid graph state is **unreachable**, not merely unlikely. Any code path
that can produce an invariant violation is a kernel bug, not a usage error.
These eight invariants are the entire point of the project — they are never
traded away for convenience, performance, or compatibility.

### II. Structural Safety Over Defensive Checks

Invalid states MUST be structurally unrepresentable at the type level. Runtime
guards are only acceptable for conditions that cannot be expressed by the type
system (e.g., cross-process invariants enforced at the syscall dispatch
boundary).

- Kernel fields (`owner`, `refcount`, `content_hash`, provenance stamps) are
  owned exclusively by the kernel and MUST NOT be settable from userspace.
- No attribute namespace key may shadow or alias a kernel field.
- Userspace MUST NOT be able to construct a node in an invalid starting state.

**Rationale**: Defensive runtime checks are a symptom of types that admit
invalid representations. Types MUST make the right state the only state.

### III. Simplicity as Correctness

Every component MUST be as simple as the requirements allow. Complexity is a
liability: it hides bugs, resists testing, and compounds over time.

- Implementations MUST NOT add abstraction layers not demanded by current
  requirements (YAGNI applies without exception).
- The journal is a canonical example of this principle: append-only log, one
  commit record, nothing more.
- Prefer explicitness over cleverness. When two approaches work, choose the
  more obvious one.
- Complexity added beyond what the current requirements demand MUST be justified
  via the Complexity Tracking table in the plan.

**Rationale**: Simplicity is a correctness strategy. The simpler a subsystem,
the more obviously correct it is and the easier it is to verify under fault
injection.

### IV. Exhaustive Testing Discipline (NON-NEGOTIABLE)

All kernel-level and node-store code MUST be covered by:

- **Property-based fuzzing** using `proptest` or equivalent.
- **Fault injection tests** covering simulated I/O failures and power-loss
  recovery scenarios.
- **100% branch coverage** — no branch in invariant-critical paths may be
  unexercised.

New invariant-critical code MUST be test-driven: tests MUST be written and
confirmed to fail before the implementation is written (Red-Green-Refactor).
A new property that cannot currently be tested MUST be logged as a known gap
and prioritised in the next cycle.

**Rationale**: The entire value proposition of LOB is structural correctness.
If the test suite cannot detect an invariant violation, that guarantee is
marketing, not engineering.

### V. no_std Kernel Core

The `lobns` crate MUST remain `no_std`. All types in the core node-store API
MUST be constructable without the OS allocator or standard library.

- Every `unsafe` block MUST carry a `// SAFETY:` comment explaining precisely
  which invariant justifies the unsafe operation.
- Every `unsafe` block requires explicit reviewer sign-off in the PR.
- Code requiring `std` MUST live in a separate crate or behind a feature flag.

**Rationale**: `no_std` enforces portability and keeps the kernel's dependency
surface minimal. It also prevents accidental reliance on OS facilities that
LOB itself is designed to replace.

### VI. Provenance Integrity

Kernel provenance fields (`created_by_process`, `created_by_binary`,
`created_by_user`) MUST be stamped by the kernel at node creation time and
MUST NOT be modifiable by userspace under any circumstances.

- `created_by_user` is NEVER `None`; kernel-created nodes use `UserId(0)`.
- `created_by_binary` MUST persist after the creating process exits and remain
  permanently resolvable via the binary node on disk.
- Any API path that would allow a caller to supply or override provenance values
  MUST be rejected at design time.

**Rationale**: Unforgeable provenance is a security guarantee. A compromised
process MUST NOT be able to attribute its actions to a different binary or user.
An audit trail with no tamper protection is not an audit trail.

### VII. Rust-Native Design

LOB is implemented in Rust because Rust's ownership type system is isomorphic
to the ownership semantics the kernel enforces. This isomorphism MUST be
exploited, not worked around.

- Invariants that can be expressed at compile time MUST be expressed at compile
  time (e.g., typed lease handles for `ref`/`ref_mut` rather than runtime flags).
- Re-implementing ownership semantics via raw pointers or runtime reference
  counting when a safe Rust abstraction is available is a constitution violation.
- Choosing a different implementation language requires a MAJOR constitution
  amendment with full rationale.

**Rationale**: Using anything other than Rust would mean reimplementing a worse
version of the type system before the actual work could begin.

## Technical Standards

- **Language**: Rust, stable channel. Nightly features require written
  justification in the PR and a migration plan for when they stabilise.
- **Build System**: Cargo workspace; each crate has its own `Cargo.toml`.
- **Core Crate**: `lobns` — `no_std`, pure library, zero OS dependencies.
- **Testing Stack**: `cargo test` + `proptest` (property-based) + fault
  injection harness.
- **POSIX Layer**: `libposix` userspace shim only; the kernel has no POSIX
  knowledge and MUST NOT acquire any.
- **Unsafe Policy**: Every `unsafe` block carries a `// SAFETY:` justification
  comment and requires reviewer sign-off.
- **Dependency Policy**: Prefer zero-dependency or well-audited crates. Each
  new dependency requires explicit justification ("why this crate, why now")
  in the PR description.

## Development Phases

- **Phase 0** *(current)*: Proof of concept on Linux/Windows. `lobns` as a
  pure `no_std` library with exhaustive testing (property fuzzing, fault
  injection, 100% branch coverage).
- **Phase 1**: Node browser and `lob-shell` built on top of `lobns`.
- **Phase 2**: Kernel integration; `libposix` compatibility layer; POSIX
  programs running unmodified.
- **Phase 3**: Bootable OS image; hardware driver layer.

Work MUST be sequenced within the current active phase. Implementing Phase N+1
concerns before Phase N is complete is a constitution violation unless
explicitly sanctioned as a time-boxed research spike with a written outcome.

## Governance

- This constitution supersedes all other project guidelines, docs, and
  conventions on any point where they conflict.
- Amendments to a Core Principle require:
  1. Documented rationale for the change.
  2. A migration plan for existing code if semantics change.
  3. A version bump per the policy below.
- **Versioning Policy**:
  - MAJOR: Removal or redefinition of a Core Principle, or removal of a
    Governance rule.
  - MINOR: New principle or new section added, or materially expanded guidance.
  - PATCH: Clarifications, wording fixes, typo corrections, non-semantic
    refinements.
- All PRs touching kernel-level or node-store code MUST include a
  Constitution Check confirming no invariant enforcement is weakened.
- Complexity beyond what requirements demand MUST be justified in the PR via
  the Complexity Tracking table in the implementation plan.

**Version**: 1.0.0 | **Ratified**: 2026-03-18 | **Last Amended**: 2026-03-18
