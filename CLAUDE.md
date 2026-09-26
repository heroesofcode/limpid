# Limpid

A system cleaner and disk-space analyser for Linux, written in Rust.

The goal is not to ship another cleaner. It is to be **the best disk cleaner
in the Linux ecosystem** — safer, faster and more honest than every incumbent,
and the obvious choice on Arch/Omarchy before it is the obvious choice
anywhere else.

Read these before changing anything:

- @.claude/mission.md — what winning means, and who we are measured against
- @.claude/safety.md — the invariants. Breaking one is never a trade-off worth making.
- @.claude/responsive-ui.md — the layout contract. **Every** layout change must satisfy it.
- @.claude/architecture.md — the crate map, and what may depend on what
- @.claude/roadmap.md — where the project is now and what comes next

## The three properties that are never traded away

**1. Safe.** An app that deletes files gets exactly one chance to be wrong.
Dry run is the default, every path is re-checked against the filesystem at the
moment it is acted on rather than when the plan was made, and no path ever
crosses the privilege boundary. If a change makes Limpid more capable and less
certain, the change is wrong. See @.claude/safety.md — those rules are
load-bearing, not aspirational.

**2. Fast and efficient.** A scan of a real home directory is hundreds of
thousands of files. Walks are parallel, measured once, and never redone for
data nobody is looking at. Long work goes to a blocking thread; the frame loop
never stalls. Prefer a bounded heap over sorting everything, an atomic over a
mutex, and one level of a tree over the whole tree.

**3. Responsive.** Limpid is built for a tiling window manager. It does not
choose its own size and can be handed a quarter of a small screen without
warning. **No layout may assume a width.** Every view takes `layout::Metrics`
and asks it. This is a hard requirement, not a polish item — see
@.claude/responsive-ui.md for the contract and the checklist you must run
before calling any layout work finished.

## Target platforms, in priority order

1. **Arch Linux / Omarchy / Hyprland.** First-class, and the reference
   platform. Everything is validated here first: pacman and AUR caches, btrfs
   with `compress=zstd`, snapper snapshots, the Omarchy theme, tiling layout.
2. **Other tiling compositors and Wayland desktops.** Should already work; do
   not add anything that assumes a floating window or a fixed size.
3. **The major distributions** — Fedora, Debian/Ubuntu, openSUSE. Supported,
   but the distro-specific scanners come after Arch is excellent. Never write
   a scanner that silently produces wrong numbers off Arch; produce nothing
   instead.

## Working here

```sh
make build     # cargo build --release --workspace
make check     # fmt, clippy with warnings denied, and the tests
```

`make check` must pass before anything is considered done. CI runs the same
three steps and treats warnings as errors.

Point the whole engine at a fixture instead of the real filesystem with
`--root`, or `LIMPID_ROOT`:

```sh
cargo run -p limpid-cli -- scan --root /tmp/fixture
```

Note the gap documented in @.claude/roadmap.md: `--root` does **not** yet
sandbox the privileged helper. Never run `clean --apply --include-root`
against a fixture expecting it to be inert.

## Conventions

- `#![forbid(unsafe_code)]` in every crate. There is no reason for this
  program to contain unsafe code, and no exception is worth the argument.
- `#![warn(missing_docs)]` in the library crates. Every public item is
  documented.
- **Comments explain why, not what.** The codebase's existing comments are the
  standard: they record the reasoning, the alternative that was rejected, and
  the failure the code is guarding against. Match that density and tone. A
  comment restating the line below it is noise; a comment saying why
  `st_blocks` rather than `len()` is the whole point.
- **Test names are sentences describing behaviour.**
  `a_symlink_planted_in_a_cache_is_not_followed_out`, not `test_symlink`.
  Read the existing names before adding one.
- Every behavioural claim in the README is backed by a test. If you change
  behaviour, change the test and the README together.
- Commits are Conventional Commits — `release-please` builds the changelog and
  the version bumps from them. `feat:`, `fix:`, `chore:`, `docs:`.
- **Never reference AI, assistants or agents in anything written.** Not in
  commit messages, not in pull request titles or bodies, not in issues, not in
  the changelog, not in code comments. No `Co-Authored-By` trailer naming an
  assistant, no "generated with" footer. The project's history reads as the
  work of its authors, because that is what it is. Describe what a change
  does, never who or what produced it.
