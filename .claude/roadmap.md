# Roadmap to 1.0

What a user can *do* with Limpid, release by release. The README's phase
table records what has been built; this records what to build next.

## Where we stand

The engine is ahead of the field — copy-on-write accounting, the two-tree
browser scan, a guard that re-checks at the moment of action, a privilege
split where no path crosses the boundary. What is behind is how much of that
engine a user can reach.

Missing today, and present in every competitor:

- The storage view is **read-only**. You can see a 4 GB file and do nothing.
- No duplicate detection.
- No concept of old or unused files.
- No history, no undo.
- No exclusions.
- No config file at all — settings persist nothing.
- Scan progress is a spinner: no percentage, no current path, no cancel.
- No search or filter.

---

## 0.3 — Act from the storage view

- Select files and directories in the treemap and the largest-files list
- Move to trash — the default
- Delete permanently, behind a second confirmation
- Open containing folder, copy path
- Exclude a finding from future scans
- **A second safety model for this.** `Guard` is an allowlist of boundaries;
  the storage view shows arbitrary paths outside all of them. Deleting from
  there needs a denylist instead: nothing outside `$HOME`, nothing the system
  needs, one path per action, never a path the user has not seen on screen.
  Settle this before drawing any button.
- Fix: `--root` does not sandbox the privileged helper
- Fix: the browser-running check is never re-checked at removal time
- Fix: `Guard::check` only tests the leaf of a path for being a symlink
- Fix: stale-result race on the storage page — harmless today, destructive
  once this release lands

## 0.4 — Trust and memory

- A config file. The project has none today.
- A managed exclusion list, honoured by every scanner and by the storage view
- History of every run: what was removed, how big, where it went, when
- Undo for anything that went to the trash
- Show the exact file list before acting. The confirmation shows only totals
  today, while the README says you see the exact list.
- Sanity check on magnitude: refuse to proceed quietly when a plan would
  remove an implausible share of the disk
- Render `Guard::boundaries()` in Settings. The safety model is the product's
  best argument and it is currently invisible.
- Fix: `describe_from_mountinfo` discards the whole mount table when any line
  fails to parse — the `?` operators sit inside the loop. Silent, and it costs
  the btrfs caveat.
- Fix: `remove_coredumps` follows symlinks, contrary to its own comment
- Fix: `Breakdown::loose` is accumulated and then unconditionally zeroed
- Fix: `sha256sums=('SKIP')` in the PKGBUILD

## 0.5 — Generic findings

None of these need per-application knowledge, so they work on every
distribution on day one.

- Duplicate files. Group by size, compare a head block, hash in full only for
  what survives.
- Large and old files, ranked by access time. "47 files you have not opened in
  two years, 12 GB" is a sentence no Linux cleaner currently produces.
- Empty directories
- Broken symlinks

## 0.6 — Correct accounting

- **Exclusive bytes** — how much a removal would *actually* free. Hardlinks
  are already handled and snapshots are detected, but reflinks between live
  files and shared extents after dedup are still counted twice. A file sharing
  extents with another live file frees nothing when deleted. Needs `FIEMAP` or
  btrfs qgroups.
- Verify the result: `statvfs` before and after, reported against the promise.
  "Freed 3.2 GB. Predicted 3.4 GB; the difference is held by 2 snapshots."
- Account for 100% of the disk: files, snapshots, swapfile, hibernation image,
  journal, btrfs metadata, unallocated. Every analyser answers the file
  question and leaves the user confused when the numbers do not add up.
- Investigate whether `same_file_system(true)` stops early at btrfs subvolume
  boundaries — subvolumes carry their own `st_dev`. Unverified; confirm before
  acting on it.
- Benchmark against `gdu` in CI. Being slower than a pure analyser is a
  regression, not an acceptable cost of knowing more.

## 0.7 — Reclaim without deleting

Three levers that free space with zero data loss. No cleaner offers any of
them. Depends on 0.5 for duplicates and on 0.6 to prove the gain.

- Reflink dedup: duplicates share extents instead of one copy being removed.
  On a reflink-capable filesystem this is the **default** action for
  duplicates, with deletion as the alternative.
- btrfs recompression. Omarchy mounts root with `compress=zstd:3`, but files
  written before that stay uncompressed and `/home` may not be compressed at
  all. A targeted `btrfs filesystem defragment -r -czstd`.
- Snapshot thinning as a named privileged operation — "keep the N most
  recent", "drop older than D days", never "delete this path". Often the
  difference between freeing 2 GB and 40 GB on a machine with snapper.

## 0.8 — Coverage

- **A declarative catalog in TOML.** Adding an application costs a Rust PR
  today, so the long tail never arrives. A target becomes data: name, paths,
  kind, risk, explanation. The rule that makes it safe: a declarative entry
  may only name paths **inside boundaries that already exist**, and may not
  create one. Coverage grows to hundreds of entries with zero increase in
  risk surface.
- The long tail of applications, on top of that format
- Orphan packages (`pacman -Qtdq`), per-package confirmation. The README
  promises this twice and no scanner exists.
- Uninstall leftovers: config and cache belonging to packages no longer
  installed
- `.pacnew` assistance — show the diff, offer to launch `pacdiff`
- Flatpak unused runtimes and old deployments
- Steam and Proton shader caches
- Docker and Podman with image and layer sizes shown before authorising

## 0.9 — Maintenance and reach

- Automatic policies applied by a systemd user timer: "keep the pacman cache
  at 3 versions, the journal at 14 days"
- A trend chart. Where the space is going over time is a more useful question
  than where it is now.
- Group findings by application. Users think in applications, not
  directories.
- Scan progress and cancel
- Search and filter
- Portuguese first, then other languages
- Keyboard navigation and accessibility across every view
- Other distributions: apt/dnf/zypper. Only where Limpid can be as correct as
  it is on Arch — produce nothing rather than a wrong number.
- Nix store garbage collection
- Trash on other mounts (`$topdir/.Trash-$uid`)

## 1.0

A promise about stability, not a feature. Ship it when:

- Everything above is done, or cut and removed from the README
- Every claim in the README is backed by a test, and none describes something
  unimplemented
- Config and history formats are versioned and migratable
- Releases are signed, with real checksums in the PKGBUILD
- A fresh install has been run end to end on Arch, Fedora and Debian
- The full checklist in @.claude/responsive-ui.md passes on every view, at
  every band, including a short window
- The helper has a written security review. The privilege split is the best
  argument the project has; make it legible to someone who has not read the
  code.

---

## Decisions taken

**0.5 before 0.6.** Generic findings ship before exclusive-byte accounting.
They are independent, they are the features users recognise as "a real
cleaner", and accounting is long work that should not block visible progress.
The cost is that 0.5 reports ordinary sizes rather than exclusive ones.

**The declarative catalog stays in 0.8, before the long tail.** Writing fifty
targets by hand and then migrating them to TOML is work thrown away.

## One distinction that must not be lost

There are two allowlists in this project and they are opposites.

| | `caches.rs` → `KNOWN` | `guard.rs` → `boundaries` |
|---|---|---|
| What it is | coverage | safety |
| Today | 11 entries | 12 entries |
| A short list means | an incomplete product | a safe system |
| Should grow | a great deal | only by deliberate edit in Rust |

The `caches.rs` allowlist is correct design, not a limitation — plenty of
applications keep non-regenerable state under `~/.cache`, so "delete
everything not known to be precious" is the wrong default. The problem is
only that growing it is expensive, which is what 0.8 fixes.
