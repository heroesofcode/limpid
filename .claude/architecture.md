# Architecture

```
src/                   the desktop application (Iced)
crates/limpid-core     scanning, classification, execution
crates/limpid-theme    palette resolution and live theme reload
crates/limpid-cli      headless front-end, used by the test suite
crates/limpid-helper   privileged helper, invoked through polkit
```

The root package is the application, and the libraries are workspace members.
That is not just tidiness: `release-please`'s Rust support needs the
configured package path to be a real manifest, and a virtual workspace at the
root gives it nothing to update or derive a version from.

## Dependency rules

- **Nothing that deletes a file lives in the GUI.** The GUI builds a `Plan`
  and hands it to `limpid-core`. If you find yourself writing
  `std::fs::remove_*` under `src/`, the design has gone wrong.
- **What runs as root stays small enough to read in one sitting.**
  `limpid-helper` depends on `limpid-core` only for the `privileged` module —
  the request and report types. It must not grow a dependency on the
  scanners, the guard, or the walker.
- `limpid-theme` knows nothing about cleaning. `limpid-core` knows nothing
  about colours.

## The pipeline

```
catalog::scan  ->  Scan { Category { Target } }     a proposal, never an action
      |
   Selection                                        what the user said yes to
      |
Plan::from_targets  ->  Plan { items, operations }  items have paths;
      |                                             operations never do
      +---> Executor (unprivileged)  -> Guard::check per path, at the moment
      |
      +---> Runner -> pkexec -> limpid-helper       named operations only
```

The separation between `Target` and `Plan` is load-bearing: the executor
takes a plan and nothing else, so there is no path from "found something" to
"removed it" that skips an explicit selection.

## `Roots` — why everything is testable

`crates/limpid-core/src/paths.rs`. Every path a scanner uses is resolved
through `Roots`, so the whole engine can be pointed at a fixture directory.
Without it, testing a cleaner means either mocking the filesystem or letting
the suite loose on the developer's real home directory.

`LIMPID_ROOT` (or `--root`) sets it. In that mode `XDG_*` variables are
deliberately ignored, so a fixture is not perturbed by the developer's own
environment.

**Use `Roots::under(tempdir)` in every test that touches the filesystem.**
There is no excuse for a test that reads `$HOME`.

## Key modules in `limpid-core`

| Module | What it owns |
|---|---|
| `paths` | `Roots`. Where to look. The reason tests are possible. |
| `walk` | Parallel measurement on `ignore::WalkBuilder`. Hardlinks counted once, symlinks never followed, `.snapshots`/`.zfs` never descended. |
| `size` | `Size { apparent, on_disk }`. `on_disk` is `st_blocks * 512` and is the only number that predicts `df`. |
| `volume` | Filesystem facts that change what a byte count *means*: CoW, compression, snapshots, capacity. |
| `model` | `Scan`/`Category`/`Target`. Proposals. |
| `catalog/*` | One module per category of thing worth looking for. Scanners only measure. |
| `browser` | Browser discovery, profiles, and whether one is running. |
| `analyse` | The "where did it go" side: one-level breakdown and largest files. Knows nothing, finds everything. |
| `plan` | `Plan`, `Disposal`, `Selection`. What the user agreed to. |
| `guard` | The last check before anything is removed. See @.claude/safety.md. |
| `execute` | Carrying out a plan. |
| `privileged` | The boundary types and the `Runner`. **No path crosses this.** |

## The application

- `app.rs` — state, messages, `update`, the top-level `view`. Long work goes
  to `tokio::task::spawn_blocking`; the frame loop never stalls.
- `layout.rs` — `Metrics`. See @.claude/responsive-ui.md. Every view takes it.
- `style.rs`, `typography.rs` — the palette applied, and the type scale
  (taken from Omarchy's own `shell.toml` so text matches the bar it shares a
  screen with).
- `view/*` — one module per page. Each takes `(palette, metrics, state)`.
- `widget/*` — the gauge and the treemap, both Iced canvases.

## Theme resolution

`limpid-theme` reproduces Omarchy's own alias and derivation cascade and
agrees with `omarchy-theme-color` across the installed themes. Off Omarchy it
follows `org.freedesktop.appearance` for light/dark and uses its own palette.

The live reload watches the **parent** directory, not `colors.toml` and not
`current/theme`. `omarchy-theme-set` does `rm -rf current/theme && mv
next-theme current/theme`, so a watch on the inode stops firing the first
time the theme changes. This is the single subtlest thing in the crate and
there is a test that reproduces the swap exactly.
