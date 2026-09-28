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
- **No coverage of project build artifacts.** Measured on the reference
  machine on 2026-09-26: **17.04 GiB**, which was 48% of the home directory,
  and Limpid reported none of it. `catalog/development.rs` covers
  `~/.cargo` and `~/.npm`; nothing covers a project's own `target/` or
  `node_modules/`. This is the largest single gap by measured bytes.
- No concept of old or unused files.
- No history, no undo.
- No exclusions.
- No config file at all — settings persist nothing.
- Scan progress is a spinner: no percentage, no current path, no cancel.
- No search or filter.

---

## 0.3 — Act from the storage view

- Select files and directories in the treemap and the largest-files list
- Move to trash — the default, and correct here for a reason that inverts
  the Overview's: this is the user's own data, and trashing a 4 GB ISO is a
  rename, so it is instant and reversible. The Overview deletes caches
  outright because trashing them frees nothing; both rules follow from the
  same question, asked about different content.
- Delete permanently, behind a second confirmation
- Open containing folder, copy path
- Exclude a finding from future scans
- **A second safety model for this.** `Guard` is an allowlist of boundaries;
  the storage view shows arbitrary paths outside all of them. The distinction
  that resolves it: the guard exists to protect against *the program* being
  wrong, and a path the user pointed at is not a guess. So an explicitly
  chosen path is its own permission category — everything that does not
  depend on guessing still applies (nothing outside `$HOME`, no symlink,
  never `$HOME` itself, never `.ssh`), but the boundary list stops being the
  criterion. One path per action, and never a path the user has not seen on
  screen. Settle this before drawing any button.
- Files only. Selecting a *directory* in the treemap is a much larger blast
  radius and waits for history and undo in 0.4.
- Fix: `--root` does not sandbox the privileged helper
- Fix: the browser-running check is never re-checked at removal time
- Fix: `Guard::check` only tests the leaf of a path for being a symlink
- Fix: stale-result race on the storage page — harmless today, destructive
  once this release lands

## 0.4 — Trust and memory

Deliberately before 0.5. Everything Limpid removes today is regenerable
cache; being wrong costs a re-download. From 0.5 it touches photographs and
documents. The trust infrastructure has to exist before the first feature
that can lose something irreplaceable.

- A config file. The project has none today.
- A managed exclusion list, honoured by every scanner and by the storage view
- History of every run: what was removed, how big, where it went, when
- Undo for anything that went to the trash
- Directory selection in the storage view, now that there is an undo
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
distribution on day one. This is also where the largest measured win lives.

### Project build artifacts

17 GiB on the reference machine, and none of it visible today. Two rules make
it safe, and both were established by measurement rather than by reasoning:

- **Detect by sibling marker, never by name.** `Cargo.toml` → `target/`,
  `package.json` plus a lockfile → `node_modules/`, `pyproject.toml` →
  `.venv/`, `go.mod`, `build.gradle`. The 2026-09-26 sweep turned up
  `~/.cargo/registry/src/…/cc-1.5.1/src/target`, which is **source code**. A
  name-only rule deletes it.
- **Risk follows the age of the *project*, not the artifact** — the date of
  the git HEAD. A `target/` in a repository untouched for a year is free
  money; yesterday's is not. Stale for months is `Safe`, recent is not
  offered at all.

The Go module cache is stored read-only, so a recursive delete fails
part-way; it needs `go clean -modcache` and is therefore an operation, not a
path.

### Duplicate files

Group by size, compare a head block, hash in full only for what survives,
and `memcmp` before offering anything. Three constraints, all confirmed on
real data here:

- **A hardlink is not a duplicate.** `target/debug/limpid` and its twin under
  `deps/` were the same inode (232634, two links). A naive finder reports
  437 MiB and deleting one frees zero.
- **Equal size is not equal content.** Three `.7z.00N` volumes were 995 MiB
  each to the byte, with different hashes. Only the hash separates them.
- **Never offer every copy in a group.** Default to keeping the oldest.

Disposal is the trash, not deletion — this is user data.

Do not size this feature from the reference machine. It has 17 files over
100 KB across Pictures, Downloads and Documents; it cannot contain duplicates
and says nothing about a machine that can.

### Also

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
- The same work turns the snapshot caveat from a warning into a number.
  Saying "2.4 GB of this is pinned by 6 snapshots" is the difference between
  a caveat and an answer.
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

- Reflink dedup (`FIDEDUPERANGE`): duplicates share extents instead of one
  copy being removed. Both files stay, the space comes back. On a
  reflink-capable filesystem this is the **default** action for duplicates,
  with deletion as the alternative. Czkawka deletes; this is strictly better.
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
- Do not freeze the format until it has had users. At 1.0 it becomes a
  contract.
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

The release that decides whether anyone outside Arch ever runs this.

- Automatic policies applied by a systemd user timer: "keep the pacman cache
  at 3 versions, the journal at 14 days"
- **Scan-only timer plus a notification** above a threshold. Universal, and
  it is what turns Limpid from an application someone remembers to open into
  one that tells them. The Omarchy bar widget is a layer on top of it, not
  the feature.
- A trend chart. Where the space is going over time is a more useful question
  than where it is now.
- Group findings by application. Users think in applications, not
  directories.
- Scan progress and cancel
- Search and filter
- Portuguese first, then other languages
- Keyboard navigation and accessibility across every view. The reason is
  accessibility, not tiling-window-manager taste.

### Other distributions

Per @CLAUDE.md, these come after Arch is excellent, and the rule holds:
produce nothing rather than a wrong number. But "after" is not "never" — the
package-manager category is Arch-only today, which means on Ubuntu Limpid
misses the thing that matters most.

- `apt` — `/var/cache/apt/archives`
- `dnf` — `/var/cache/dnf`
- `zypper` — `/var/cache/zypp`
- **Old snap revisions.** Snap keeps previous revisions by default and this
  is routinely several GB on an Ubuntu desktop — the direct equivalent of the
  pacman cache, and probably the single largest win on that distribution.
- Nix store garbage collection
- Distro detection, so a Fedora machine is not shown pacman targets
- Trash on other mounts (`$topdir/.Trash-$uid`)

### Packaging

Packaging *is* distribution for a Linux tool. A program one click away gets
orders of magnitude more users than one that has to be built.

- **Flathub.** The default store in GNOME Software and KDE Discover across
  Ubuntu, Fedora and Mint. Needs appstream metainfo, screenshots and review.
- **The Flatpak build cannot do system-level cleaning.** A sandboxed
  application cannot install a polkit policy on the host. So the Flatpak is
  user-level only and the native packages do everything. That has to be
  stated in the description, not discovered.
- `.deb` and `.rpm` after that. Distribution repositories come later and
  usually arrive on their own once a tool has traction.

## 1.0

A promise about stability, not a feature. Everything above is the *feature*
work, and finishing it produces a 0.9, not a 1.0 — the remaining gap is
evidence, and evidence is not something that can be programmed.

Ship it when:

- Everything above is done, or cut and removed from the README
- Every claim in the README is backed by a test, and none describes something
  unimplemented
- Config, history and catalog formats are versioned and migratable
- Releases are signed, with real checksums in the PKGBUILD
- A fresh install has been run end to end on Arch, Fedora and Debian, and on
  ext4 as well as btrfs. Everything to date has run on one machine, one
  filesystem, one desktop.
- **The privileged path has been exercised end to end.** It has been built
  and its refusals tested; it has never been watched working. That alone
  blocks 1.0.
- The full checklist in @.claude/responsive-ui.md passes on every view, at
  every band, including a short window
- The helper has a written security review. The privilege split is the best
  argument the project has; make it legible to someone who has not read the
  code.
- Real users have run it for real weeks

**Veto: any unresolved report of data loss postpones 1.0**, however finished
everything else is. In this category reputation is the product — it is what
separates BleachBit from the registry cleaners nobody installs.

---

## Decisions taken

**0.4 before 0.5.** History and undo ship before duplicates and before
project artifacts. Both of those touch files a person, not a program, would
have to recreate, and the first release that can lose something irreplaceable
should not also be the first release that can explain what it did.

**0.5 before 0.6.** Generic findings ship before exclusive-byte accounting.
They are independent, they are the features users recognise as "a real
cleaner", and accounting is long work that should not block visible progress.
The cost is that 0.5 reports ordinary sizes rather than exclusive ones.

**The declarative catalog stays in 0.8, before the long tail.** Writing fifty
targets by hand and then migrating them to TOML is work thrown away.

**Duplicate detection stays in scope.** It was briefly cut on the grounds
that a sweep of the reference machine found nothing. That sweep was
worthless: the machine has 17 files over 100 KB across Pictures, Downloads
and Documents. It is also a feature both CCleaner and CleanMyMac ship, and
"use czkawka instead" contradicts the premise that scanning, classifying and
acting belong in one pass.

**Measure here to confirm and to find traps; never to size demand.** The
reference machine is the machine of the people building the tool, and it is
the least representative one available.

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

The `guard.rs` list is the opposite, and 0.3 and 0.5 both need paths it
cannot enumerate in advance. Two additions, and they are different in kind:
a **predicate** rule for what the program discovers on its own ("a directory
named `target` whose parent holds a `Cargo.toml`"), and an **explicit
choice** category for what the user points at. Getting either wrong once
contaminates every release after it.
