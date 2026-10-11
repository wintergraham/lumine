# Styling retained views

lumine's Rust API combines GPUI/QuickGUI-style fluent methods with owned component children. Import `Styled` to use the methods on a `View` or reusable `Styles` patch. Values are logical pixels except explicit percentage dimensions; spacing methods take pixel values directly, not a numbered spacing scale.

```rust
use lumine::compose::{button, column, text};
use lumine::style::{rgb, Styled, Styles};

let card = column()
    .w(320.)
    .p(16.)
    .gap(12.)
    .bg(rgb(0x202838))
    .rounded(10.)
    .border_1()
    .border_color(rgb(0x465268))
    .text_color(rgb(0xf0f4ff))
    .text_size(16.)
    .child(text("Connection settings").text_size(22.))
    .child(button()
        .px(14.).py(8.)
        .bg(rgb(0x2854a0))
        .hover(|s| s.bg(rgb(0x3266bd)))
        .active(|s| s.bg(rgb(0x1d407e)))
        .focus(|s| s.border_1().border_color(rgb(0x9abfff)))
        .disabled_style(|s| s.opacity(0.45))
        .child(text("Connect"))
        .on_click(|| println!("connect")));
```

Mount with `Ui::mount(card)` or use the native window's view rendering entry point. A background, rounded border, or shadow decorates the container itself; it does not add a layout child. Children remain retained when a visual style changes.

## Properties and patches

| Area | Methods |
| --- | --- |
| Dimensions | `w`, `h`, `size`, `w_percent`, `h_percent`, `w_full`, `h_full`, `min_w`, `max_w`, `min_h`, `max_h` |
| Padding | `p`, `px`, `py`, `pt`, `pr`, `pb`, `pl` |
| Margins | `m`, `mx`, `my`, `mt`, `mr`, `mb`, `ml` |
| Layout | `flex_row`, `flex_col`, `gap`, `flex_grow`, `flex_shrink`, `grow`, `shrink_0` |
| Alignment | `items_start`, `items_center`, `items_end`, `items_stretch`, `justify_start`, `justify_center`, `justify_end`, `justify_between`, `justify_around` |
| Surface | `bg`, `rounded`, `border`, `border_1`, `border_color`, `shadow`, `shadow_none` |
| Text | `text_color`, `text_size`, `line_height`, `line_height_normal`, `letter_spacing`, `text_wrap`, `font_family`, `font_weight`, `font_bold`, `italic` |
| Effects | `opacity`, `blur`, `edge_fade`, `translate`, `isolated` |
| Clipping | `overflow_hidden`, `overflow_visible` |

`Styles::new()` is an empty patch. `.style(patch)` changes only fields supplied by that patch. Later writes win, including individual edges: `.p(8.).px(12.).pt(3.)` produces top 3, right 12, bottom 8, left 12. A later `.pl(20.)` preserves the other three edges. Per-edge padding participates in intrinsic sizing, flex allocation, and resizing.

```rust
use lumine::style::{rgb, Styled, Styles};
let surface = Styles::new().p(12.).bg(rgb(0x202838)).rounded(8.);
let compact = surface.clone().py(6.); // retains horizontal padding and surface
```

Use `.apply(...)` for reusable style functions, `.when(...)` for a conditional transformation, and `.when_some(...)` for an optional value. These helpers work on both views and sparse `Styles` patches; skipped closures are never called.

```rust
use lumine::compose::{column, text};
use lumine::style::{rgb, Styled, Styles};

fn card_surface<S: Styled>(style: S) -> S {
    style.p(16.).bg(rgb(0x202838)).rounded(10.)
}

let compact = true;
let width = Some(320.);
let card = column()
    .apply(card_surface)
    .when(compact, |view| view.py(8.))
    .when_some(width, |view, width| view.w(width))
    .child(text("Connection settings"));
let reusable_patch = Styles::new().apply(card_surface);
```

These decisions run once when constructing the description. They do not subscribe to signals or reapply after mounting. For styles that change with state, return a patch from `reactive_style`; the same helpers can be used inside that tracked callback.

Text color, size, wrapping, font selection, line height and letter spacing inherit through children and components. A child can override each field independently. Updating an inherited value updates the affected text properties; it does not rerun component construction. Explicit overrides shield descendants from unchanged effective typography. Other style properties are local rather than inherited. Padded or decorated text mounts a retained box around its text node; plain text stays a single node. Static and reactive wrapping propagate through boxed text and reflow its retained text node when the available width changes.

## Percentage dimensions

`.w_percent(50.)` takes half of the parent's definite content width; `.h_percent(50.)`
does the same for height. `.w_full()` and `.h_full()` mean 100 percent. These
methods take percentages (50 means 50%, not 0.5), while `.w()` and `.h()` retain
their logical-pixel units. The parent's padding is excluded from the reference
size; the resulting child size includes the child's own padding. Margins and gaps
are separate, so two 50-percent children plus a gap can overflow unless flex
shrink is enabled.

```rust
use lumine::compose::prelude::*;
let content = row().w_full().h(240.).p(16.)
    .child(column().w_percent(40.).h_full().child("Navigation"))
    .child(column().w_percent(60.).h_full().child("Content"));
```

Pixel and percentage values replace one another per axis, including sparse style
patches and interaction variants. `.w(200.).w_percent(50.)` uses 50 percent;
a subsequent `.w(80.)` restores a pixel width. Removing a reactive override restores
the base style. Resizing a parent recomputes dependent child allocation without
reconstructing components or losing editor state. Images, ordinary scrollers and
virtual-list viewports accept the same dimension methods.

A percentage reference is definite when its parent dimension is explicit,
resolved from a percentage, or allocated by flex/stretch (the root uses the
window viewport). A mere available-space constraint or min/max bound does not
make an intrinsic axis definite. On indefinite axes, percentages fall back to
automatic intrinsic sizing, including ordinary cross-axis stretch; layout does
not iterate to solve cyclic percentage sizes. Absolute children use their
parent's final content box because they do not contribute to its intrinsic size.
For example, percentage height inside the content of a vertical scroller usually
has an indefinite reference, while its viewport can have a definite height.

Flex grow/shrink allocation and logical-pixel min/max constraints still apply.
Values above 100 percent are allowed; negative and non-finite values normalize to
zero. Only width and height currently expose percentage units; percentage
padding, margins and min/max dimensions are not provided.

## Image fitting

Image views accept `.object_fit(ObjectFit::...)`, with centered placement inside
the content box after padding:

| Mode | Behavior |
| --- | --- |
| `Fill` (default) | Stretch independently to content width and height |
| `Contain` | Preserve aspect ratio and show the full image |
| `Cover` | Preserve aspect ratio, fill the box and clip excess pixels |
| `None` | Keep source pixel dimensions, centered and clipped |
| `ScaleDown` | Contain without scaling above the source dimensions |

`.object_contain()` and `.object_cover()` are shortcuts. This property is local
to the image and is not inherited by children. Sparse reactive and interaction
patches follow the same precedence as other style properties; removing an
override restores the underlying mode. Uncovered content shows the view's
background. Clipping is rectangular, including when the outer background is
rounded. These modes change placement, not the image's intrinsic size contribution.

```rust
use std::sync::Arc;
use lumine::{compose::prelude::*, image::ImageData};
let pixels = Arc::new(ImageData::new(2, 1, vec![255; 8]).unwrap());
let thumbnail = image("Preview", pixels)
    .size(160., 120.).p(8.).bg(rgb(0x202838)).object_contain();
```

## Line height

`.line_height(24.)` sets the distance between visual text rows to 24 logical pixels.
It inherits through ordinary children, components and providers, including boxed
labels and editors. Font size still controls glyph size. `.line_height_normal()`
explicitly resets an inherited pixel value to normal spacing for the child's own
font size. Omitting the field from a `Styles` patch preserves inheritance; removing
a dynamic override restores the underlying base or inherited value.

```rust
use lumine::compose::prelude::*;
let content = column().text_size(16.).line_height(24.)
    .child(text("First line\nSecond line"))
    .child(text("Compact caption").text_size(12.).line_height_normal());
```

Positive finite values are used exactly, including values smaller than the font
size. Tight spacing can overlap adjacent glyph rows; glyph ink outside the text
node's allocated rectangle is clipped, consistent with other text rendering.
Zero, negative and non-finite values resolve to normal spacing. The normalized
metric supports stable equality and hashing, so repeated equivalent styles do
not invalidate layout or paint.

Reactive line-height changes update retained text geometry and editor caret,
selection, hit testing and IME placement together. They do not reconstruct the
component or modify its text model. Single-line inputs still use the selected
line height for their caret and metrics while remaining unwrapped.

## Letter spacing

`.letter_spacing(2.)` adds two logical pixels of tracking to native glyph
advances. It inherits through children, components and lexical slots, including
editors. `.letter_spacing(0.)` explicitly resets inherited tracking; omitting the
field preserves inheritance. Signed values allow tighter or looser text.

```rust
use lumine::compose::prelude::*;
let heading = column().letter_spacing(2.)
    .child(text("STATUS"))
    .child(text("Normal caption").letter_spacing(0.));
```

Native shaping uses the same pixel-to-em conversion for measurement, editor
geometry, GPU placement and software rasterization. Tracking is part of the font
cache key. Changing it reflows text and updates carets, selections and hit testing
without rebuilding components or changing editor text. Equal normalized values
do not invalidate geometry. Integer tracking changes can reuse glyph bitmaps
while updating their positions.

The native shaper applies tracking per shaped glyph, including the final advance;
ligatures and combining sequences therefore follow its glyph clustering rather
than a per-character CSS spacing algorithm. Core-only fallback measurement uses
approximate grapheme cells and clamps their advances at zero for very tight
spacing. Negative native advances can overlap glyphs; selection rectangles retain
ordered edges, and text ink still clips to its allocated bounds.

Zero and negative zero are equivalent; non-finite values normalize to zero.
Finite values clamp to ±1,000,000 logical pixels to keep extreme tracking from
producing nonfinite geometry. This is a bound on the style value, not a promise
that extreme spacing is readable. Font-aware custom shapers receive the
normalized value through `FontStyle::letter_spacing`.

## Interaction and reactive styles

Styles resolve in this order: base fluent properties, `reactive_style`, hover, active, focus, disabled. Later patches take precedence only for fields they specify. While disabled, hover, active, and focus patches are excluded. Removing a state or a dynamic property restores the underlying style; it does not leave stale values behind.

`.disabled(true)` and `.disabled_when(|| signal.get())` disable interaction and update accessibility. `.disabled_style(...)` controls appearance; supplying that patch alone does not disable a view. Focus is cleared when a focused subtree becomes disabled. Buttons support pointer and keyboard activation.

```rust
use lumine::{compose::column, style::{rgb, Styled, Styles}, widgets::Ui};
let mut ui = Ui::new(640., 480.);
let highlighted = ui.signal(false);
let read = highlighted.clone();
let view = column().p(12.).reactive_style(move || {
    Styles::new().bg(if read.get() { rgb(0x345888) } else { rgb(0x202838) })
});
let mounted = ui.mount(view);
highlighted.set(true);
```

Signal reads inside `reactive_style` are tracked. A background or text-color change damages paint without invalidating layout. Size, spacing, wrapping, or font-size changes invalidate dependent layout. Translations and opacity use compositor invalidation. Equal resolved properties produce no new scene mutation. The mounted children and component-local state are preserved in each case.

## Transparency and effects

`.translate(x, y)` uses logical pixels. Each nonfinite coordinate (NaN or either
infinity) resolves to zero at the scene boundary; the other coordinate remains
independent. Repeating an equivalent normalized translation leaves geometry
revision, layout and damage unchanged. Removing a sparse override restores the
base translation. This does not clamp otherwise finite coordinates.

Effect values normalize at the scene boundary before equality and damage checks.
Opacity clamps to `[0, 1]`; NaN uses the default `1`, while positive/negative
infinity clamp to `1`/`0`. Blur radius and edge-fade length map nonfinite or
negative values to `0`. An explicit invalid value still overrides an earlier
style: it resolves to this fallback rather than restoring inheritance. Repeating
the same normalized effect does not add scene damage or invalidation.

`rgb(0xRRGGBB)` creates an opaque color; `rgba(0xRRGGBBAA)` includes alpha. `.opacity(value)` multiplies opacity. Without isolation, opacity applies through individual subtree primitives: overlapping translucent children can accumulate alpha. Use `.isolated(true).opacity(0.5)` when the subtree should first render as a group and then composite once. GPU isolated layers retain their pixels across eligible transform/opacity-only updates, subject to cache limits.

`.blur(radius)` is backdrop blur, not a generic image or text blur. It samples content painted behind the view. GPU backdrop updates reconstruct the affected filter outputs and their sampling halos in paint order. Unrelated damage stays local; connected filters can still require the full target. The software reference backend conservatively repaints the full target when blur is present. The GPU blur implementation caps sigma at 64 physical pixels. `.edge_fade(length)` fades the top and bottom edges. `.overflow_hidden()` clips to the rectangular layout box; a rounded background does not imply rounded clipping of all descendants.

The [native effects example](../crates/lumine-desktop/examples/effects.rs) builds its glass card and controls with component children, `.reactive_style(...)`, and these fluent effect methods. Component-owned tasks animate a translation signal; the application does not append or mutate scene nodes. Run it with `cargo run -p lumine-desktop --example effects`.

`.shadow(BoxShadow { ... })` adds an outer shadow without changing layout dimensions. Shadow extents participate in damage tracking. `.shadow_none()` explicitly clears an inherited patch's shadow setting (shadows do not inherit down the view tree).

The native GPU renderer shapes text and provides fallback fonts. Font family, weight and italic inherit through components and slots alongside color, size and wrapping. Use `.font_family("DejaVu Serif").font_weight(700).italic(true)`, or the portable `FontFamily::{SansSerif, Serif, Monospace}` choices. Weight is clamped to 1–1000; named families and unavailable faces use platform fallback. Native measurement, caret shaping and GPU glyph selection share font attributes; changing them invalidates layout and retained glyph geometry. The software reference backend also renders styled fonts, using a lazy platform font database. Letter spacing and custom line height use the shared inherited metrics described above. It does not yet expose the full CSS layout or styling surface of the inspiration projects. GPU cache and effect limits are documented in [the renderer notes](../crates/lumine-gpu/README.md).

The software styled-font cache retains at most 128 entries and 8 MiB of text keys/glyph images; oversized runs render without being retained. Its platform font database is created on first styled text and shared with software layer renderers. Headless core-only scenes use approximate metrics until a `Scene::set_font_text_shaper` backend is installed.

Editable views use the same fluent styles. For example, `text_input("Search", query).w(320.).px(12.).font_family(FontFamily::Monospace).focus(|s| s.border(2.).border_color(rgb(0x5ea5ff)))` styles the actual focusable editor root. Its text and caret inherit color, size, family, weight, and italic; root padding and allocated dimensions determine the clipped editing viewport. Typography and dimension changes preserve selection and the bound model. Use `.text_wrap(true)` on `text_area` to wrap at its allocated content width, including inherited or reactive wrapping styles. Rendering, pointer hit testing, selection, caret movement and IME geometry share the same wrapped layout. Resizing reflows visual lines without adding newlines to the model or losing selection. Wrapping defaults to false, preserving horizontal scrolling for long lines; `text_input` always remains a single unwrapped line. Home/End follow visual lines while wrapping; repeated Up/Down retain the preferred horizontal caret position across shorter lines. Edits, pointer placement, Home/End, and changes to wrapping, font metrics or allocated width reset that preferred position.

Multiline editors accept wheel scrolling without moving the selection; scrolling clamps to the text extent and bubbles to ancestors at an exhausted edge. Single-line editors leave vertical wheel events to their parent. Font/color changes preserve component ownership; color-only changes repaint without relayout, while font changes also update selection, hit testing and caret metrics.

`.absolute()` removes a child from normal layout flow. It starts at its parent's
content origin (inside padding), with `.ml(...)` and `.mt(...)` setting offsets.
It does not consume flex space, add a gap, or enlarge the parent's intrinsic size.
Its wrapping constraints follow the parent's allocated content dimensions.
`.relative()` restores ordinary row, column, or overlay participation, including
when applied by a reactive style patch. Absolute positioning is relative to the
immediate parent; it does not implicitly create a window-level portal or change
paint order. Use `.translate(...)` for a compositor-only positional change when
layout participation should stay the same.


Focused editors reuse bounded shaped geometry for unchanged displayed text and
metrics. Selection, scrolling and navigation do not reshape that text; font,
line-height and wrapped-width changes invalidate the reuse entry. See
[editor layout reuse](performance/editor-shaping/README.md) for admission limits
and measured call counts. This does not change component ownership or the style
inheritance rules above.
