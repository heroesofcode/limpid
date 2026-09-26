# Safety invariants

An app that deletes files gets exactly one chance to be wrong. These are not
guidelines. If a change cannot be made without breaking one of them, the
change does not get made.

Every rule here is currently backed by tests. If you change the behaviour,
the test that proves it must change in the same commit — and if you find
yourself deleting a test rather than adapting it, stop and reconsider the
change.

## 1. Dry run is the default

`Executor::dry_run` and `Executor::applying` are separate constructors on
purpose (`crates/limpid-core/src/execute.rs`). There is no boolean anyone can
pass the wrong way round. The destructive path has to be asked for by name,
at the call site.

A dry run and a real run must agree on the figure. That is a test, not a
hope: `a_dry_run_and_a_real_run_agree_on_the_figure`.

## 2. Every path is checked at the moment it is acted on

Not when the plan was built. The world changes in between, and an attacker or
a careless `mv` only needs that window.

`Guard::check` (`crates/limpid-core/src/guard.rs`) runs immediately before
each removal, against the filesystem as it is then. A path is removable only
if it:

- is absolute,
- contains no `..` component (rejected, never resolved — resolving means
  canonicalising, which follows symlinks, which is exactly the escape),
- is not a protected name,
- sits strictly inside a boundary, and is not a boundary itself,
- is not a symlink.

The boundary list is **written out by hand**, not derived from what the
scanners declare. Adding a scanner must not be able to widen it by accident.
A new place to clean requires a deliberate edit to `Guard::new`.

## 3. No path crosses the privilege boundary

This is the one that keeps the root-side small enough to read in one sitting.

The helper (`crates/limpid-helper`) accepts a fixed set of **named operations
with bounded parameters** — "keep the three newest versions of each package",
never "remove these files". There is no request that means "remove this
path", therefore there is no request that can be bent into meaning "remove
that one".

The helper contains **no filename parsing, no symlink resolution and no
safety judgements**. Those live on the unprivileged side, where being wrong
is survivable.

Rules for anything touching this boundary:

- Do not add a field to `Request` or `Operation` that carries a path,
  a glob, a pattern, or anything a path can be reconstructed from.
- Parameters are validated on **both** sides. The helper's copy is the one
  that counts; the caller's only exists to fail earlier and avoid a pointless
  auth prompt.
- Subprocesses run with `env_clear()` and a fixed `PATH`. Inheriting an
  environment across a privilege boundary is how a helper ends up running
  someone else's `paccache` as root.
- Where a maintained tool exists, run it rather than reimplementing it.
  `paccache` knows what a correct retention policy is and ships with pacman.
  A version of it inside the helper would be a second, worse opinion.
- `docker system prune` never gets `--volumes`. A volume can be the only copy
  of a development database.

## 4. Removal matches what is being removed

- **Caches, build output, package archives → deleted outright.** Sending a
  cache to the trash moves the bytes to another directory on the same
  filesystem, frees nothing, and fills the one place the user looks to
  recover things.
- **Anything a person rather than a program would have to recreate → the
  freedesktop trash.**
- The confirmation says which, in those words. `Disposal::describe`.

## 5. Directories are emptied, not removed

Applications expect their cache directory to exist and misbehave quietly when
it does not. An empty directory costs nothing.

## 6. Some files are protected by name, wherever they appear

`PROTECTED_NAMES` in `guard.rs`. `Local State` above all: it holds
`os_crypt.encrypted_key`, the wrapped key every saved cookie and password is
encrypted with. Removing it leaves every row intact and **permanently
undecryptable** — worse than losing the rows.

This list exists because browser boundaries have to cover a whole profile
directory (profile names are not knowable in advance), and that directory
holds irreplaceable things next to caches. Adding a browser boundary means
checking whether its credential store needs a new entry here.

## 7. Browsers must be closed — a refusal, not a warning

Found by walking `/proc/*/fd`, not by reading a lock file: a lock file
survives a crash and would report a browser that is not running. Descriptors
cannot lie.

Cleaning a live Chromium profile does not free the space while the
descriptors are open, and can make it discard the whole database rather than
the part you asked for.

> **Known gap.** This is currently checked at scan time only, never
> re-checked at the moment of removal — the one safety rule that does not yet
> follow rule 2. See @.claude/roadmap.md.

## 8. No blanket rules

- Removing orphan packages asks per package — never as one "remove all
  orphans" action. (Not yet implemented; this is the rule it must follow when
  it is. See @.claude/roadmap.md.)
- `.pacnew` files are reported as work to do, never as reclaimable space.
  Deleting one unmerged means silently running without a change upstream
  thought necessary.
- Nothing that needs a human decision is ticked by default.

## Checklist for adding a scanner

A scanner is a list of paths and an explanation. It must not know how to
delete anything.

1. Does it need a new `Guard` boundary? If so, add it to `Guard::new`
   deliberately, and add a test that the boundary itself is refused.
2. Does the new tree hold anything irreplaceable? If so, add it to
   `PROTECTED_NAMES` with a comment saying what is lost.
3. Is the `Kind` right? `Disposal::for_kind` decides delete-vs-trash from it.
4. Is the `Risk` right? `Safe` means *regenerates itself with no
   user-visible consequence*. A slow first launch is `Review`.
5. Does it need root? Then it needs a named `Operation`, or it must not be
   offered at all — `Selection::includes` drops a privileged target with no
   operation, because offering it would be a lie.
6. Write the test against a fixture via `Roots::under`. The suite never reads
   the developer's real home directory.
