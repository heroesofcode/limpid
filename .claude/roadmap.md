# Where Limpid is, and what comes next

## Current state

Phases 0–6 and 8–9 of the README roadmap are done. As of this writing the
workspace is healthy: `cargo fmt --check` clean, `cargo clippy --workspace
--all-targets --all-features -- -D warnings` clean, and **216 tests + 2
doctests passing**.

What already works, and works well:

- Measurement that distinguishes apparent size from `st_blocks`, with
  hardlinks counted once and snapshots never descended into.
- A guard that is a genuine second opinion, checked at the moment of action.
- A privilege split where no path crosses the boundary — the strongest part
  of the design, and the thing to protect above all else.
- Browser scanning that finds the `~/.config` half other tools miss.
- A treemap analyser that knows nothing and finds everything.
- Live Omarchy theming, including the directory-swap problem nobody else
  would have got right first try.
- A responsive layout system with measured breakpoints and tests.

The foundation is better than the incumbents' already. What follows is what
turns that into "the best cleaner in the ecosystem".

---

## Tier 0 — correctness gaps to close before adding features

These came out of a full read of the codebase. None is catastrophic; all of
them are places where the code does not yet hold a promise the project makes
elsewhere. Fixing them is cheap now and expensive later.

**1. `--root` does not sandbox the privileged helper.** *(highest priority)*
`Runner::new()` is called unconditionally in `limpid-cli/src/main.rs` and
`src/app.rs`. The helper has no notion of `Roots`, so `clean --root
/tmp/fixture --apply --include-root` runs `paccache` and `journalctl
--vacuum-time` **against the real system**. The README explicitly offers
`--root` as a way to satisfy yourself about what Limpid does before letting
it near your home directory — today that promise is false for the privileged
half. `Roots::is_sandboxed()` already exists and is currently dead code
outside its own tests; it is exactly the guard required.

**2. The browser-is-running check is never re-checked at removal time.**
`holder_of` runs once, during the scan. A user can scan, open Brave, then
click Clean. This is the only safety rule that does not follow the "checked
against the filesystem as it is now" discipline that `guard.rs` argues for.

**3. `Guard::check` only tests the leaf for being a symlink.** An
intermediate component that is a symlink escapes the boundary: with
`~/.config/chromium/Default/Cache` symlinked elsewhere, the target
`Cache/Cache_Data` is not itself a link, passes the guard, and `read_dir`
follows the path. Top-level targets are safe (a symlinked `~/.cache/yay` is
refused); nested browser targets are not. Walk the components from the
boundary down.

**4. One malformed line in `/proc/self/mountinfo` discards the whole parse.**
In `volume.rs::describe_from_mountinfo` the `?` operators sit inside the
`for` loop, so an unexpected line returns `None` from the function and throws
away the `best` match already found. The failure is silent and lands on the
flagship feature: no `Volume` means `caveats()` emits no btrfs or compression
warning, and the user gets a promise of gigabytes with the caveat missing.
`continue` instead of `?`.

**5. `remove_coredumps` follows symlinks, contrary to its own comment.**
`helper/main.rs` says "symlink_metadata, so a symlink is removed as a link"
but calls `entry.metadata()`, which follows. The outcome is still safe
(`remove_file` never follows), but a symlink to a directory is skipped and a
broken one errors out, so dumps accumulate. In a binary that runs as root, a
comment describing a guarantee the code does not provide is the kind of thing
that ages badly. Use `entry.file_type()`.

**6. `Breakdown::loose` is accumulated and then unconditionally zeroed.**
`analyse.rs` sums into `breakdown.loose` and three lines later assigns
`Size::ZERO`. The field is always zero. Either drop the field and the
accumulation, or drop the reset — as it stands it is dead work plus a public
field that lies.

**7. Stale-result race on the storage page.** `Message::Explored` stores the
survey without checking that it corresponds to `trail.last()`. Two quick
clicks and the breadcrumb shows one directory while the treemap shows
another. Carry the path in `Explored` and discard what does not match.

**8. Firefox profiles with `IsRelative=0`.** `profiles.ini` may carry an
absolute `Path=`; `self.config.join(absolute)` discards the base. The guard
then refuses the path, so it is a silent disappearance rather than damage —
but the profile is invisible with no explanation. `parse_profiles_ini` does
not read `IsRelative`.

**9. `sha256sums=('SKIP')` in the PKGBUILD.** A remote tarball with no
integrity verification, for a package that installs a polkit helper. AUR
guidelines want a real checksum for non-VCS sources.

**10. The README promises orphan packages; no scanner exists.** "Neither runs
`paccache`, surfaces orphan packages…" and "The pacman cache, AUR build
trees, orphan packages, `.pacnew` files…" are both in the README, and
`packages.rs` has no orphan target. Either build it (see Tier 2) or stop
claiming it until it exists. Shipping a claim the code does not back is
exactly the credibility Limpid is trying to take from the incumbents.

### Performance, same tier

- `analyse::largest_files` locks a global mutex **once per file visited**.
  On a home directory of several hundred thousand files this serialises much
  of the parallel walk. An atomic "current threshold" checked before taking
  the lock would eliminate almost every acquisition.
- `Treemap::update` re-runs `squarify` and reallocates every tile label on
  **every mouse event**, including plain movement. Cache the layout against
  the bounds.

---

## Tier 1 — the differentiators that actually win the market

This is the part that makes Limpid the best rather than merely the
best-engineered. Ranked by how much ground each one takes.

### 1. Snapshot-aware reclaim, and snapshot thinning

**This is the single biggest win available, and it is Arch/Omarchy-specific.**

On an Omarchy machine with snapper, the space is very often *in the
snapshots*, and no cleaner addresses it. Limpid already detects that
snapshots exist and correctly says the reclaim may be deferred — but it stops
at the caveat.

Two parts:

- **Honest accounting.** Report what a removal will actually free given the
  snapshots pinning it. Doing this exactly needs btrfs qgroups, which are off
  by default and expensive; a good approximation plus an honest label beats
  everyone else's silence.
- **Offer to thin the snapshots.** As a named privileged operation, in the
  existing boundary style: "keep the N most recent snapshots", "drop
  snapshots older than D days". Never "delete this snapshot path" — the same
  discipline as `TrimPackageCache`.

Nothing in the ecosystem does this. For the reference platform it is often
the difference between freeing 2 GB and freeing 40 GB.

### 2. Reclaim without deleting — reflink deduplication

On btrfs and XFS, identical extents can be shared instead of removed.
`duperemove` and `rmlint` do this; **no general-purpose cleaner offers it**.

It is also perfectly aligned with the project's own values: it frees space
with *zero* data loss, which makes it the safest possible reclaim. "Limpid
freed 6 GB and deleted nothing" is a headline no competitor can print.

Pairs naturally with phase 7 (duplicate files): find duplicates, then offer
dedup *or* removal, with dedup as the default on a reflink-capable
filesystem.

### 3. Verify what was actually reclaimed

Take `statvfs` before and after, and report the delta against what was
promised. This turns the copy-on-write caveat from an excuse into a
measurement, and it is a trust-builder nobody else has:

> Freed 3.2 GB. Predicted 3.4 GB; the difference is held by 2 snapshots.

It also makes the CoW claim in the README self-verifying, and would catch a
whole class of regression automatically.

### 4. An undo journal

Record what was removed, when, how big it was, and where it went. For trashed
items offer restore; for deleted caches, at least an auditable log. "I can
see exactly what it did" is the answer to the single biggest objection to
installing a cleaner.

### 5. Per-application attribution

Users think in applications, not directories. "Brave — 3.4 GB across three
trees" is a more useful sentence than four separate cache rows, and the
scanner already knows the grouping. This is mostly a presentation change over
data Limpid already has, which makes it unusually cheap for the impact.

---

## Tier 2 — breadth, in rough priority order

Arch/Omarchy first, per @.claude/mission.md.

- **Orphan packages** (`pacman -Qtdq`), per-package confirmation, as the
  README already promises.
- **Flatpak**: unused runtimes and old deployments — frequently several GB,
  and `~/.var/app` caches alone (already scanned) are only part of it.
- **Old rustup toolchains**, `sccache`/`ccache`, `~/.cache/uv`.
- **Docker/Podman** beyond the existing prune: image and layer reporting, so
  the user sees the size before authorising.
- **Steam / Proton shader caches** — routinely tens of gigabytes on a gaming
  machine, and safely regenerable.
- **Electron application caches** generically, using the Chromium two-tree
  knowledge Limpid already encodes.
- **Systemd**: `systemd-tmpfiles --clean`, and per-unit log sizes.
- **Trash on other mounts** — `$topdir/.Trash-$uid`, not just the home trash.
- **Nix store** garbage collection, for the users who have one.
- Only then: apt/dnf/zypper for the other distributions.

---

## Tier 3 — the things that make it stick

- **A scheduled measurement** via a systemd user timer, recording a daily
  total. "Your caches grew 4 GB this week" turns a tool you run once into one
  you keep installed. No telemetry — local only, per @.claude/mission.md.
- **Keyboard navigation and accessibility** across every view.
- **A proper "what is actually using my disk"** that includes the non-file
  answers: snapshots, the swapfile, the hibernation image, journal size,
  btrfs metadata and unallocated space. Every analyser in the ecosystem
  answers the file question and leaves the user confused when the numbers do
  not add up.
- **Packaging reach** once Tier 0 and Tier 1 land: AUR is the beachhead, then
  Flatpak, then COPR/PPA.

---

## How to use this document

Tier 0 before Tier 1. A cleaner that is fast and clever and occasionally
wrong loses to one that is slower and never wrong — and Limpid's entire
pitch is that it is the careful one. Items 1, 2 and 3 in Tier 0 are places
where the code does not hold a promise the README makes out loud, and that is
the worst kind of bug this project can have.
