# Retained component layout

Build children with `div`, `row`, `column`, and reusable functions or components. Layout is a style refinement; it does not require appending scene commands:

```rust
use lumine::{compose::prelude::*, layout::ContentAlign};
let dashboard = div().grid().grid_cols(4).grid_rows(3)
    .gap_x(12.).gap_y(8.).w_full().h(240.)
    .child(div().col_span_full().child(text("Header")))
    .child(div().row_span(2).child(text("Sidebar")))
    .child(div().col_span(3).row_span(2).child(text("Main")));
let cards = row().w_full().flex_wrap()
    .gap_x(10.).gap_y(16.).align_content(ContentAlign::Start)
    .children((0..30).map(|i| div().w(140.).h(44.).child(text(format!("Item {i}")))));
```

Grid tracks are equal `minmax(0, 1fr)` shares. Starts use CSS-style one-based grid lines; negative starts count backward. `col_span`, `row_span`, and their `_full` variants compose with starts. Flex supports wrapping/reverse wrapping, reverse direction, basis, grow/shrink, item self-alignment and line distribution. Insets accept `Length::Px`, `Length::Percent`, or `Length::Auto`; a plain float converts to pixels. `Length::Percent(1.)` means 100%, whereas convenience methods such as `p_percent(10.)` take percentages from 0 to 100.

Percent padding uses the containing block's width on every edge. Auto margins absorb remaining space. `aspect_ratio` supplies the missing dimension. Pixel refinements override earlier percentage spacing, gaps and size constraints on the corresponding properties. `.hidden()` removes a subtree from layout and interaction while retaining its component identity; `.invisible()` preserves allocation. `.flex()` restores display and `.visible()` restores visibility. Hidden focused controls blur, and visibility observers stop hidden media deadlines.

Scrollable views forward container layout to their retained content, so `scroll(offset).grid().grid_cols(2)` and `scroll(offset).flex_row().flex_wrap()` use the same child architecture. The viewport retains its own padding, dimensions and clipping. `overflow_x_hidden` and `overflow_y_hidden` independently clip paint, hit testing and visibility; the corresponding `_visible` methods restore overflow on that axis.

Ordinary row/column/overlay layouts keep the existing small cached layout path. Advanced containers allocate one retained Taffy 0.9 tree for their immediate children. Stable child IDs and Taffy caches survive resize and content updates; removed children release cache entries. Layout options use an optional shared allocation, so default styles do not allocate an extended options block. An unchanged scene performs no layout work. See [the feature cost audit](performance-feature-audit.md) for default-path and cache bounds.

Run `cargo run -p lumine-desktop --example layout` for the resizable gallery. `cargo test -p lumine --test advanced_layout` covers geometry, refinement precedence, visibility and scrolling; the core retained-cache regression checks allocation reuse and stale-child removal. These tests establish the documented cases, not universal browser CSS conformance.
