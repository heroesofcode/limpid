# Roadmap to 1.0

This is a feature roadmap: what a user can *do* with Limpid, release by
release, until it is the one people recommend. The README's phase table
records what has been built; this records what to build next and why that
order.

## Where we stand

Phases 0–6 and 8–9 of the README table are done. The workspace is healthy:
`fmt` clean, `clippy -D warnings` clean, **216 tests + 2 doctests passing**.

The engine is genuinely ahead of the field — copy-on-write accounting, the
two-tree browser scan, a guard that re-checks at the moment of action, a
privilege split where no path crosses the boundary. What is behind is the
**product**: how much of that engine a user can actually reach.

The honest summary of today's gap:

| | Today |
|---|---|
| Overview | Scan, tick, clean. Good. |
| Storage | **Read-only.** You can see a 4 GB file and do nothing about it. |
| Duplicates | Not started. |
| Old / unused files | No concept of them. |
| Undo / history | None. Once it is gone, there is no record it existed. |
| Exclusions | None. No way to say "never touch this". |
| Settings | Shows the palette. Stores nothing — the project has no config file at all. |
| Scan progress | A spinner. No percentage, no current path, no cancel. |
| Search / filter | None. |

Five of those nine are things every competitor has.

## Competitive scorecard

Where Limpid stands against the tools a user would otherwise combine. `~`
means partial.

| | Limpid now | Limpid 1.0 | BleachBit | Stacer | czkawka | baobab/ncdu | qdirstat |
|---|---|---|---|---|---|---|---|
| Classifies what a file *means* | yes | yes | yes | ~ | no | no | ~ |
| Honest on btrfs / CoW | yes | yes | no | no | no | no | no |
| Arch-native (pacman, AUR, pacnew) | ~ | yes | no | no | no | no | no |
| Browser `~/.config` half | yes | yes | no | no | no | no | no |
| Privilege split, no path to root | yes | yes | no | no | n/a | n/a | no |
| Treemap / space analysis | yes | yes | no | no | no | yes | yes |
| **Delete from the analyser** | **no** | yes | n/a | n/a | yes | no | yes |
| **Duplicate files** | **no** | yes | no | no | yes | no | no |
| **Old / unused files** | **no** | yes | no | no | no | no | no |
| **Undo / history** | **no** | yes | no | no | ~ | n/a | no |
| **Exclusions** | **no** | yes | yes | no | yes | n/a | yes |
| **Scheduling** | **no** | yes | no | no | no | no | no |
| Reclaim without deleting (reflink) | no | yes | no | no | no | no | no |
| Snapshot-aware | ~ | yes | no | no | no | no | no |

The four columns of `no` in the middle are the product gap. The two rows at
the bottom are where nobody is, which is where 1.0 wins.

---

# 0.3 — Act on what you can see

**The biggest single gap, and the one users will hit first.** The storage
view answers "where did it go" and then abandons you. Every analyser worth
using lets you act on a finding; `qdirstat` and `czkawka` both do.

### Features

- **Select entries in the treemap and the largest-files list.** Single and
  multiple.
- **Move to trash** from the storage view. The default, always.
- **Delete permanently**, behind a second confirmation, for things too large
  to trash usefully.
- **Open containing folder** and **copy path**. Often the right answer is
  "let me look at this myself", and refusing to help with that is a worse
  outcome than a delete button.
- **Exclude from future scans**, straight from the finding.
- **Recheck a level** after acting, without re-walking the whole tree.

### The safety design this needs first

This is the hard part, and it must be settled before any button is drawn.
`Guard` is an **allowlist of boundaries** — the storage view, by design,
shows arbitrary paths outside all of them. Deleting from there cannot go
through the same door.

Proposed second mode, to be reviewed against @.claude/safety.md:

- **Trash is the only default.** Permanent delete is opt-in per action, never
  a saved preference.
- **A denylist, not the boundary allowlist**: refuse anything outside
  `$HOME`, refuse dotfile config directories, refuse anything the running
  system needs. `/`, `/etc`, `/usr`, `/boot`, `~/.ssh`, `~/.gnupg`, the whole
  `PROTECTED_NAMES` list.
- **One path per action, no recursion into the unknown.** A directory
  selected in the treemap is trashed as a unit, after showing the file count
  and size it is about to move.
- **Never act on a path the user has not seen on screen.**
- The unprivileged executor only. Nothing in the storage view may reach the
  helper.

### Also in 0.3 — safety debt that blocks everything after it

These are places where the code does not hold a promise the README makes out
loud. They are cheap now and expensive once features pile on top.

1. **`--root` does not sandbox the privileged helper.** `Runner::new()` is
   called unconditionally, and the helper has no notion of `Roots`, so
   `clean --root /tmp/fixture --apply --include-root` runs `paccache` and
   `journalctl --vacuum-time` against the real system. The README offers
   `--root` as the safe way to try Limpid before letting it near your home.
   `Roots::is_sandboxed()` already exists and is dead code outside its own
   tests — it is exactly the guard required.
2. **The browser-running check is never re-checked at removal time.**
   `holder_of` runs once, at scan. Scan, open Brave, click Clean. The only
   safety rule that does not follow the discipline `guard.rs` argues for.
3. **`Guard::check` only tests the leaf for being a symlink.** An
   intermediate symlinked component escapes the boundary. Top-level targets
   are safe; nested browser targets are not.

---

# 0.4 — Find what no scanner was written for

The catalogue can only find what someone anticipated. These four findings
need no per-application knowledge at all, which means they work on every
distribution on day one — and they are where the surprising gigabytes are.

### Features

- **Duplicate files.** README phase 7, still open. Hash-on-demand: group by
  size, then compare a head block, then hash in full only for what survives.
  Never compare whole files across a whole home directory.
- **Reclaim duplicates without deleting — reflink dedup.** On btrfs and XFS,
  share the extents instead of removing a copy. `duperemove` and `rmlint` do
  this; **no general-purpose cleaner offers it.** It is the safest possible
  reclaim — zero data loss — and it is the single most defensible headline
  the project can print: *freed 6 GB, deleted nothing*. On a reflink-capable
  filesystem this should be the **default** action for duplicates, with
  deletion as the alternative.
- **Old and unused files.** Ranked by access time, scoped to the usual
  suspects — `~/Downloads` above all. "47 files you have not opened in two
  years, 12 GB" is a sentence no Linux cleaner currently produces.
- **Empty directories and broken symlinks.** Cheap to find, satisfying to
  clear, and `rmlint`-class findings that make Limpid the one tool instead of
  two.

### Why this order

0.4 is what makes Limpid worth switching *to* rather than worth trying.
Duplicates alone is the reason a large fraction of `czkawka`'s users
installed it.

---

# 0.5 — Trust

Everything here exists to answer one objection: *how do I know what it did?*
No cleaner in the ecosystem answers it well, and it is the whole reason
people distrust the category.

### Features

- **History and undo.** A local journal of every run: what was removed, how
  big it was, where it went, when. Restore anything that went to the trash,
  in one click. An auditable record for what was deleted outright.
- **Verify what was actually reclaimed.** `statvfs` before and after, and
  report the delta against the promise:
  > Freed 3.2 GB. Predicted 3.4 GB; the difference is held by 2 snapshots.

  This turns the copy-on-write caveat from an excuse into a measurement, and
  makes the README's flagship claim self-verifying. Nobody has this.
- **Show me exactly what will go.** The confirmation currently lists target
  names and totals. Add a drill-down to the actual file list, so "you see the
  exact list before anything moves" is literally true.
- **Exclusions, properly.** A managed list, editable in Settings, honoured by
  every scanner and by the storage view. Glob patterns.
- **A "what may Limpid touch" screen.** Render `Guard::boundaries()` in
  Settings. The safety model is the product's best argument and it is
  currently invisible.

### Infrastructure this forces

The project has **no configuration file at all** today. 0.5 needs one:
exclusions, history, preferences. Do it once, properly, under
`$XDG_CONFIG_HOME/limpid` and `$XDG_STATE_HOME/limpid`, versioned from the
first release so it can be migrated later.

Also fold in here:

4. **`describe_from_mountinfo` discards the whole mount table** when any line
   fails to parse — the `?` operators sit inside the `for` loop. Silent, and
   it lands on the flagship feature: no `Volume` means no btrfs or
   compression caveat. `continue` instead of `?`.
5. **`remove_coredumps` follows symlinks**, contrary to its own comment
   (`entry.metadata()` where the comment says `symlink_metadata`). Outcome is
   still safe, but in a root binary a comment describing a guarantee the code
   does not provide ages badly.
6. **`sha256sums=('SKIP')` in the PKGBUILD.** A remote tarball with no
   integrity check, for a package that installs a polkit helper. Sign the
   releases while you are here.

---

# 0.6 — Arch-native, completely

The README already claims this ground. 0.6 is where the claim becomes true
without qualification, and it is the release that makes Limpid unarguable on
the reference platform.

### Features

- **Snapshot management.** The biggest reclaim available on an Omarchy
  machine, and nobody addresses it. On a system with snapper, the space is
  very often *in the snapshots*; Limpid already detects them and stops at the
  caveat. Two parts: honest accounting of what a removal will really free
  given what is pinned, and **thinning** as a named privileged operation —
  "keep the N most recent", "drop older than D days", never "delete this
  path". Same discipline as `TrimPackageCache`.
- **Orphan packages.** `pacman -Qtdq`, per-package confirmation, never a
  single "remove all orphans" action. **The README promises this twice today
  and no scanner exists** — either build it here or stop claiming it.
- **`.pacnew` / `.pacsave` assistance.** Currently reported as work to do.
  Go further: show the diff, offer to launch `pacdiff`. Reporting a problem
  you will not help with is half a feature.
- **Uninstall leftovers.** Config and cache directories belonging to packages
  that are no longer installed. Cross-reference the pacman database against
  `~/.config` and `~/.cache`. Significant space, and nothing on Linux does it
  well.
- **Flatpak, properly.** Unused runtimes and old deployments, not just the
  `~/.var/app` caches already scanned. Frequently several GB.
- **Docker / Podman with sight.** Report images, layers and build cache with
  sizes *before* asking for authorisation, instead of a blind prune.
- **Steam and Proton shader caches.** Routinely tens of gigabytes on a gaming
  machine, and safely regenerable.

---

# 0.7 — Something you keep installed

Everything so far makes Limpid worth running. This makes it worth keeping.

### Features

- **Scheduled measurement.** A systemd user timer recording a daily total.
  "Your caches grew 4 GB this week" turns a tool you run once into one that
  earns its place. Local only — no telemetry, ever, per @.claude/mission.md.
- **Trends.** A simple history chart in the overview. Where the space is
  going over time is a different and more useful question than where it is
  now.
- **Per-application attribution.** Users think in applications, not
  directories. "Brave — 3.4 GB across three trees" beats four separate cache
  rows. Mostly presentation over data Limpid already has, which makes it
  unusually cheap for the impact.
- **Progress and cancel.** Today a scan is a spinner with no percentage, no
  current path and no way out. A multi-minute walk of a large home directory
  needs all three. This is a bigger perceived-quality gap than its size
  suggests.
- **Search and filter** across findings and the storage view.

Also here:

7. **Stale-result race on the storage page.** `Message::Explored` stores a
   survey without checking it matches `trail.last()`; two quick clicks and
   the breadcrumb and the treemap disagree. Carry the path and discard what
   does not match. Fix before the page gains destructive actions in 0.3 —
   **pull this one forward.**
8. **Performance.** `analyse::largest_files` locks a global mutex once per
   file visited, serialising much of the parallel walk on a real home
   directory; an atomic threshold checked before the lock removes nearly
   every acquisition. `Treemap::update` re-runs `squarify` and reallocates
   every label on every mouse event, including plain movement.

---

# 0.8 / 0.9 — Reach

- **Internationalisation.** Portuguese first, then Spanish, German, French.
  A cleaner is a mainstream utility; English-only caps the audience well
  below "best in the ecosystem".
- **Keyboard navigation and accessibility** across every view. Full operation
  without a mouse, sensible focus order, screen-reader labels.
- **The other distributions.** apt/dnf/zypper caches, distro-appropriate log
  and package handling. Only now, and only where Limpid can be as correct as
  it is on Arch — per @.claude/mission.md, produce nothing rather than a
  wrong number.
- **Nix store** garbage collection, for the users who have one.
- **Trash on other mounts** — `$topdir/.Trash-$uid`, not only the home trash.
- **Non-file space.** Swapfile, hibernation image, journal size, btrfs
  metadata and unallocated space. Every analyser answers the file question
  and leaves the user confused when the numbers do not add up.

---

# 1.0 — Ready

1.0 is a promise about stability, not a feature. Ship it when:

- **Everything above is done**, or explicitly cut and removed from the
  README.
- **Every claim in the README is backed by a test**, and no claim describes
  something unimplemented. (Today: orphan packages, and "you see the exact
  list" showing only totals.)
- **The config and history formats are versioned and migratable.** Breaking
  a user's exclusion list in 1.1 is not acceptable.
- **Releases are signed**, with real checksums in the PKGBUILD.
- **A fresh install on Arch, Fedora and Debian** has been run end to end,
  and the distributions where a scanner would be wrong produce nothing
  instead.
- **The full responsive checklist** in @.claude/responsive-ui.md passes on
  every view, at every band, including a short window.
- **A security review of the helper**, written up. The privilege split is the
  best argument the project has; make it legible to someone who has not read
  the code.
- **Documentation for humans**, not just a README: what each finding means,
  what it costs to clear, and why it was safe to offer.

---

## Sequencing, in one line each

- **0.3** ships the missing half of a page that already exists.
- **0.4** is the reason to switch tools.
- **0.5** is the reason to trust it with a home directory.
- **0.6** makes the Arch claim unarguable.
- **0.7** is the reason it stays installed.
- **0.8/0.9** widen the audience.
- **1.0** is the promise not to break any of it.

Two rules for changing this order. Safety debt travels with the release that
makes it dangerous, never after — item 7 moves to 0.3 because 0.3 is what
makes a stale survey destructive. And no release adds a capability that the
safety model has not been extended to cover *first*; 0.3's denylist design is
the template.
