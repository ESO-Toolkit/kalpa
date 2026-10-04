# P0-A2 concrete cross-process lock consultation

You are the read-only architecture advisor for Kalpa. P0-A1 already shipped one shared crash-safe atomic publisher path-included by the Tauri and Slint crates. Choose the concrete P0-A2 cross-process read-modify-write locking dependency and API. Do not edit files.

## Finding and acceptance criteria

Atomic replacement prevents torn files but not lost updates. Both processes mutate the same `kalpa.json`, app-data `settings.json`, `kalpa-profiles.json`, and profile mirror. The OS lock must span the entire read -> mutate -> atomic write transaction.

- Same safe cross-platform protocol in Tauri and Slint.
- Canonical lock identity handles relative/absolute aliases, symlinked parents, absent target files, and Windows case/path aliases.
- Multi-lock acquisition has deterministic ordering and one bounded total deadline.
- Timeout and cancellation return actionable, user-visible errors.
- Crash releases ownership. Persistent lock-file bytes are never treated as ownership and lock files are never deleted as stale.
- Existing useful in-process mutexes remain.
- No lock is held across network calls, callbacks into UI/plugin code, directory scans, archive work, or other long operations: compute outside, lock only the short store RMW/publish.
- Tests use two real processes for block/timeout, owner kill, repeated increments/no lost updates, opposite requested ordering/no deadlock, alias identity, and both crates compiling the exact shared helper.

## Current implementation constraints and excerpts

`src-tauri/src/atomic_file.rs` is path-included by both crates and provides `atomic_write` and streaming `AtomicFile`. Publication fsyncs bytes and atomically replaces the destination. It explicitly does not serialize RMW.

Tauri has `MetadataLock(Arc<Mutex<()>>)` around metadata RMW call sites. Slint has `static METADATA_LOCK: Mutex<()>` and `metadata_guard()`. Profiles have `PROFILE_STORE_LOCK`; profile activation deliberately releases it across long folder renames, then reacquires and reloads before changing `active_profile`. Settings has `WRITE_LOCK`, a plugin in-memory cache, recovery/taint logic, and crash-atomic flush; the plugin's own non-atomic save is disabled. These process-local guards may remain but cannot substitute for OS locks.

Metadata currently exposes separate `load_metadata(addons_dir)` and `save_metadata(addons_dir, store)`. Profiles similarly load/save primary plus an app-data mirror. The two profile outputs represent one logical store and must be committed under deterministic ordered locks, without holding locks over `apply_profile`. Settings must acquire the transaction lock before reconciling the in-memory cache with disk, mutating/publishing, and release before returning to frontend/UI code.

Cargo currently has no file-lock dependency. `dunce = "1"` exists only in Tauri. Both crates already use `tempfile`. Dependencies must be added with `cargo add` in each crate; locks cannot be hand-edited.

## Candidate designs

### A. Shared `transaction_lock.rs` using `fs4`

Add `fs4` to both crates through Cargo. Path-include one module. Resolve a target identity by making it absolute, canonicalizing the nearest existing ancestor, appending unresolved components, and on Windows normalizing verbatim prefixes/case for sort/dedup. Open a persistent sibling `.<name>.kalpa.lock` with create/read/write and use `FileExt::try_lock_exclusive` in a 25ms bounded polling loop. Lock-file content has no authority and the file is never removed. RAII file handles unlock/drop; OS process death releases ownership. `acquire_many` canonicalizes, sorts, deduplicates, and uses one deadline, releasing partial acquisition on error. Return structured IO/Timeout/Cancelled errors with display text suitable for the UI. A default 2-second interactive timeout is explicit but callers may supply a test/operation timeout.

Migrate call sites so data gathering/network/archive/UI callbacks happen outside. Expose store-level short transaction helpers for metadata and profiles; settings wraps disk reload/merge/publish only. Preserve in-process mutex acquisition before the OS transaction lock so all code follows local-mutex -> sorted OS locks.

### B. `file-lock`/`fd-lock` crate with blocking locks on helper threads

Use a higher-level guard crate, but if it lacks portable nonblocking try-lock, bounded timeout/cancellation requires spawning and abandoning blocked threads or platform-specific interruption. That risks indefinite background lock waits and unclear shutdown behavior.

### C. Direct `windows` LockFileEx plus Unix `flock`

Implement platform APIs ourselves. This gives precise semantics but duplicates unsafe/OS-specific code, dependencies, error mapping, and tests in a security-sensitive foundational path despite a safe crate likely sufficing.

## Failure modes to evaluate

1. Same target addressed through relative/absolute, symlinked-parent, missing-file, `.`/`..`, Windows drive-case or verbatim aliases.
2. Two callers request A+B and B+A.
3. Timeout after acquiring the first of several locks leaks or retains it.
4. Owner is killed; contender must acquire without deleting/recreating the lock file.
5. Old PID/text remains in a persistent lock file.
6. Poll loop mistakes `WouldBlock` versus a real permission/I/O failure.
7. Lock held during a directory scan, network request, plugin callback, or profile application freezes the other shell.
8. Locking only the final atomic publish still permits both processes to read N and publish N+1.
9. Settings plugin cache was opened before another process changed disk, then flush overwrites the newer keys.
10. Primary profile save succeeds but mirror save fails; compatibility and recovery semantics must be stated without inventing a new persisted schema.

## Required output

Return only:

```text
DECISION:
1. Chosen design and numbered implementation steps

REJECTED:
1. Alternative and the concrete failure that rejects it

CRASH_RECOVERY:
1. Behavior after process kill, power loss, stale marker, timeout, or partial write

TESTS:
1. Tests distinguishing a correct design from a plausible but incorrect one

RISKS:
1. Remaining risks and required human decisions
```

## Consultation outcome — 2026-08-26

Fable chose Candidate A: one shared `transaction_lock.rs`, path-included by both
crates exactly like `atomic_file.rs`, built on `fs4` (`cargo add fs4` in both
crates; the Slint crate also needs `cargo add dunce`, which today exists only in
`src-tauri/Cargo.toml`). The lock is taken on a persistent sibling
`.<file_name>.kalpa.lock`, never on the data file — a mandatory Windows lock on
`kalpa.json` itself would break the rename publication in `atomic_file.rs` and
stall plain readers.

Requirements adopted from the review:

1. Lock identity: absolutize, `dunce::canonicalize` the nearest existing
   ancestor, re-append the unresolved components lexically normalized, and
   case-fold on Windows for sort/dedup only. Only the ancestor must exist, so a
   not-yet-created `kalpa-profiles.json` still resolves.
2. Ordering rule: in-process mutex first, then OS locks in sorted canonical-key
   order, then the RMW, then release in reverse. Never acquire an in-process
   mutex or re-enter a store helper while holding an OS lock, and never hold one
   across network I/O, directory scans, archive work, `apply_profile` renames, or
   UI/plugin callbacks. Keeping the existing local mutexes as the outer tier
   makes OS hold-time single-threaded per process.
3. `acquire_many` sorts, dedups, and shares one deadline, dropping already-held
   guards via RAII before returning `Timeout`. Poll at 25ms; only the contended
   result (`WouldBlock`) retries — every other error is a hard `Io` failure, not
   a spin.
4. Ownership is the live `flock`/`LockFileEx` state only. Lock-file bytes are
   never read, never written with authority, and the file is never deleted or
   recreated; process death releases the lock for free.
5. Metadata gets `with_metadata(addons_dir, f)` (load → mutate → save under one
   lock) replacing the ~25 `load_metadata`/`save_metadata` RMW pairs; downloads,
   hashing, and extraction are computed outside and folded in inside the closure.
   Bare reads need no OS lock.
6. Profiles need only one lock, keyed on the primary path: the app-data mirror
   path is a pure function of the same addons dir, so serializing on the primary
   serializes the mirror too — no second lock, no ordering hazard. Primary-ok /
   mirror-failed keeps today's best-effort contract and introduces no new
   persisted schema, because the mirror is consulted only when the primary and
   its `.tmp`/`.bak` artifacts are all gone.
7. Settings must take the OS lock before the taint check, then
   `reload_ignore_defaults()` to reconcile the plugin cache with disk, then
   publish, then release before returning to the frontend. Without the locked
   reload, a flush overwrites keys another process wrote after the cache opened.

Fable rejected Candidate B because neither `fd-lock` nor `file-lock` offers a
portable timed lock: a bounded timeout means abandoning a thread that still
acquires the lock afterwards and holds it with no owner until process exit. It
rejected Candidate C as a platform-specific error-mapping bug waiting to happen
in a foundational path a maintained safe crate already covers. It separately
rejected locking only the final publish (both processes still read N and publish
N+1) and any PID-liveness protocol (PID reuse plus delete-and-recreate races two
contenders onto different handles for the same identity).

The decisive test is a two-process counter: 200 locked read-increment-publish
cycles each, asserting exactly 400. A publish-only lock passes every torn-file
test and fails this one. Also required: block/timeout with a real deadline,
owner-kill recovery asserting the lock file's identity is unchanged, opposite
requested ordering, six-way alias identity, contention-vs-`Io` classification,
partial-acquisition release, settings cache reconciliation, and a compile test in
each crate proving both build the same shared file.

Open human decisions: pin `fs4` and its contended-result interpretation (the
signature changed across majors); confirm that advisory-only protection against
external writers such as Minion is in scope for P0; decide whether network
volumes degrade to a longer timeout; and confirm the 2s/25ms policy numbers,
including whether Settings gets a shorter deadline and batch operations a
per-call override.
