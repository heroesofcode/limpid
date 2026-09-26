# Mission

## The goal

Limpid should be the disk cleaner a Linux user reaches for without thinking,
the way a Mac user reaches for CleanMyMac — except that it should deserve the
reflex. That means beating the incumbents on their own ground rather than
matching them, and it means being trustworthy enough that "it deleted
something I wanted" never happens.

Concretely, Limpid wins when:

- A user on Arch can free more space with Limpid, in one pass, than with any
  combination of the existing tools they know about.
- The number Limpid promises is the number `df` shows afterwards. Nobody else
  does this on btrfs.
- It is the tool people recommend *because* it is careful, not despite it.
- It looks and behaves correctly at any width, including a quarter of a
  laptop screen in Hyprland.

## The landscape we are measured against

The Linux cleaning space splits into halves that never meet. That gap is the
opportunity, and it is worth being precise about who loses what.

### Analysers — know where the space is, know nothing about what it means

`baobab`, `ncdu`, `gdu`, `dust`, `dua`, `filelight`, `qdirstat`.

They are good, some of them are very fast, and they will happily point you at
a 4 GB directory without telling you it is a regenerable cache that will come
back tomorrow. None of them classify. `qdirstat` gets closest with
user-configured cleanup actions, but the semantics are yours to supply.

**Where Limpid must beat them:** classification, and a treemap that is at
least as fast to produce. Being slower than `gdu` at pure measurement is a
real loss, not an acceptable cost of knowing more.

### Cleaners — know the semantics, are dated and distro-blind

- **BleachBit** — the incumbent. Broad cleaner list, has a preview mode and a
  CLI. But it is GTK/Python of an older generation, its defaults are not
  distro-aware, and it does not understand that on btrfs the bytes it
  promises may not arrive.
- **Stacer** — APT-centric, and the cleaning is a side feature of a system
  monitor. Nothing in it knows what `paccache` is.
- **Sweeper** (KDE) — deliberately narrow: application and browser traces.

**Where Limpid must beat them:** being native to the machine it runs on. The
pacman cache, AUR build trees, orphan packages, `.pacnew` files, flatpak
per-app caches, snapper interactions. And being honest about copy-on-write.

### Duplicate finders — a separate tool people also need

`czkawka` (the strong modern one — Rust, GUI and CLI, also similar images and
music), `rmlint`, `jdupes`, `duperemove`.

`rmlint` and `duperemove` do something none of the cleaners do: on a
reflink-capable filesystem they **deduplicate instead of deleting**, freeing
space without losing a single file.

**Where Limpid must beat them:** not at raw duplicate detection — `czkawka`
is good and specialised. But no tool currently offers "reclaim this space
without deleting anything" inside a general cleaner, on a distro where btrfs
is the default. That is Limpid's to take.

## What "best" means, in order

1. **Safe.** Non-negotiable and first. See @.claude/safety.md. Every
   capability is evaluated against what it costs in certainty.
2. **Honest.** The figure shown is the figure reclaimed. Caveats where they
   apply, and a refusal rather than a warning where a warning would be
   ignored.
3. **Complete.** One pass finds what five tools would. A user should not need
   to know that `paccache` exists.
4. **Fast.** Measurement of a real home directory in seconds, not minutes.
   Parallel, measured once, nothing computed for a screen nobody opened.
5. **Responsive.** Correct at every width, in a tiling WM, with no horizontal
   scrollbar and no clipped control. See @.claude/responsive-ui.md.
6. **Native-looking.** Follows the Omarchy palette live; follows the
   freedesktop light/dark preference elsewhere.

## Non-goals

Worth writing down, because each of these is a plausible-sounding suggestion
that would make Limpid worse.

- **Not a system monitor.** Stacer's mistake. Disk space is the whole product.
- **Not a registry-cleaner-style "optimiser".** No placebo features, no
  "boost", no counting freed kilobytes as a score.
- **Never root for the whole application.** The UI does not run as root, and
  no amount of convenience changes that.
- **No blanket "clean everything" button** that includes anything a person
  would have to recreate by hand.
- **Not a backup tool.** Trash is a safety net for the user's own files, not
  a versioning system.
- **No telemetry.** Ever.
