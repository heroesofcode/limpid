# The responsive layout contract

**Limpid must be 100% responsive. This is a hard requirement, not polish.**

Limpid is built for a tiling window manager. On Hyprland it does not choose
its own size: opening a second window on the workspace can halve it, and a
quarter of a 1152 px laptop screen is 288 px wide. There is no minimum size
we get to declare, and no "the user can just resize it" — often they cannot,
and expecting them to is the bug.

**No layout may assume a width.** Every view takes `layout::Metrics` and asks
it.

## The rule

> Before any layout work is considered finished, it must be checked at every
> band, and it must degrade by *rearranging*, never by clipping, overflowing,
> or producing a horizontal scrollbar.

A clipped button is not a smaller button — it is one the user cannot tell is
there.

## The bands

From `src/layout.rs`. The breakpoints are set to **where something actually
stops fitting**, measured, rather than picked from a list of phone widths.
Keep it that way: if you add a breakpoint, the comment must say what stops
fitting there.

| Band | Window width | What changes |
|---|---|---|
| `Width::Tiny` | `< 520 px` | No sidebar. Rows stack. Smaller display type, tighter margins. |
| `Width::Narrow` | `520 – 760 px` | No sidebar, but rows keep two columns. |
| `Width::Wide` | `>= 760 px` | Sidebar beside the content. Everything available. |

`SIDEBAR_NEEDS = 760.0` is not a round number by accident. It is set so that
the moment the sidebar appears, what remains is *still* enough for the ring
and its figures side by side. A threshold that left the hero stacked
**because** the sidebar had just taken 216 px would be worse than having no
sidebar at all. There is a test for exactly this:
`the_sidebar_never_appears_at_the_cost_of_splitting_the_hero`.

## Ask `Metrics`, do not hardcode

| Question | Use | Never |
|---|---|---|
| Is there a sidebar? | `metrics.sidebar` | comparing a width yourself |
| Label and value on one line? | `metrics.two_columns()` | a fixed breakpoint |
| Buttons share a row? | `metrics.buttons_inline()` | guessing |
| Ring and figures side by side? | `metrics.hero_side_by_side()` | a fixed breakpoint |
| How many swatches per row? | `metrics.swatches_per_row(w, gap)` | a hardcoded count |
| Big number type size | `metrics.display()` | `ty::DISPLAY` directly |
| Page heading size | `metrics.heading()` | `ty::HEADING` directly |
| Treemap height | `metrics.treemap()` | a constant |
| Spacing | `metrics.margin`, `.gap`, `.card` | a literal |

Note that `hero_side_by_side()` asks for `gauge + FIGURES_NEED` rather than a
fixed width, so it stays correct as the gauge scales. Prefer that shape for
any new predicate: **derive the threshold from what the thing actually
costs**, so it cannot drift out of agreement with the element it governs.

`height` matters too, not just width. A wide but short window (a tiling
master/stack split) is a real case: the gauge is clamped against
`window.height * 0.32` for precisely that reason.

## The Iced trap you will hit

Documented in `src/app.rs`, and worth repeating because it has bitten this
codebase already:

> `row!` and `column!` are **Shrink** by default. A `Fill` widget inside a
> `Shrink` ancestor resolves to the content's natural width, not the
> available width. So the whole chain from the root down has to be `Fill`,
> or nothing wraps — text just runs off the edge.

If text is overflowing instead of wrapping, the bug is almost always a
missing `.width(Length::Fill)` on an *ancestor*, not on the text.

## Checklist — run this before calling any layout change done

1. **Test the bands.** Add or extend a test in `src/layout.rs` if you added a
   predicate or a breakpoint. The existing tests assert at 288, 576 and 1020
   px; use those widths.
2. **Check the degenerate case.** `Metrics::of(Size::new(0.0, 0.0))` must not
   produce a negative or zero width. There is a test:
   `a_degenerate_size_does_not_produce_a_negative_width`.
3. **Check a short window.** Not just narrow — 1200×320 is a real tiling
   layout.
4. **No horizontal scrollbar, at any width.** If one appears, something is
   `Shrink` that should be `Fill`.
5. **Nothing clipped.** Every control that exists at 1020 px must still be
   reachable at 288 px — stacked, wrapped or on its own line, but present.
6. **Run it and look.** `cargo run` and resize, or tile it. The layout tests
   prove the arithmetic; they do not prove it looks right.

## When adding a new view

- Take `metrics: Metrics` as a parameter. Every existing view does
  (`view::overview::view`, `view::storage::view`, `view::settings::view`).
- Start from the narrow arrangement and add columns as room appears, rather
  than designing wide and hoping it collapses.
- Put the breakpoint where your content stops fitting, and write the comment
  that says what stops fitting. Do not reuse an existing constant because it
  is nearby.
