//! Advanced layout is entered only for containers with opted-in properties.
//! Ordinary rows/columns retain the existing allocation/cache path.
use super::*;
use crate::layout::{ContentAlign, Display as ViewDisplay, FlexWrap as ViewWrap, Length};
use taffy::{
    prelude as t,
    style_helpers::{line, span},
};
pub(super) struct AdvancedLayoutCache {
    tree: t::TaffyTree<NodeId>,
    root: t::NodeId,
    nodes: std::collections::HashMap<NodeId, (t::NodeId, u64)>,
    order: Vec<NodeId>,
    measure_context: (Option<f32>, Option<f32>, u64),
}
impl AdvancedLayoutCache {
    fn new() -> Self {
        let mut tree = t::TaffyTree::new();
        tree.disable_rounding();
        let root = tree
            .new_leaf(t::Style::default())
            .expect("empty retained layout root");
        Self {
            tree,
            root,
            nodes: Default::default(),
            order: Vec::new(),
            measure_context: (None, None, 0),
        }
    }
}
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0. }
}
fn dim(value: Length) -> t::Dimension {
    match value {
        Length::Auto => t::Dimension::auto(),
        Length::Px(v) => t::Dimension::length(finite(v).max(0.)),
        Length::Percent(v) => t::Dimension::percent(finite(v).max(0.)),
    }
}
fn length(value: Length) -> t::LengthPercentage {
    match value {
        Length::Auto => t::LengthPercentage::length(0.),
        Length::Px(v) => t::LengthPercentage::length(finite(v)),
        Length::Percent(v) => t::LengthPercentage::percent(finite(v)),
    }
}
fn offset(value: Length) -> t::LengthPercentageAuto {
    match value {
        Length::Auto => t::LengthPercentageAuto::auto(),
        Length::Px(v) => t::LengthPercentageAuto::length(finite(v)),
        Length::Percent(v) => t::LengthPercentageAuto::percent(finite(v)),
    }
}
fn align(value: Align) -> t::AlignItems {
    match value {
        Align::Start => t::AlignItems::Start,
        Align::Baseline => t::AlignItems::Baseline,
        Align::Center => t::AlignItems::Center,
        Align::End => t::AlignItems::End,
        Align::Stretch => t::AlignItems::Stretch,
    }
}
fn content(value: ContentAlign) -> t::AlignContent {
    match value {
        ContentAlign::Start => t::AlignContent::Start,
        ContentAlign::End => t::AlignContent::End,
        ContentAlign::Center => t::AlignContent::Center,
        ContentAlign::Stretch => t::AlignContent::Stretch,
        ContentAlign::SpaceBetween => t::AlignContent::SpaceBetween,
        ContentAlign::SpaceAround => t::AlignContent::SpaceAround,
        ContentAlign::SpaceEvenly => t::AlignContent::SpaceEvenly,
    }
}
fn placement(start: Option<i16>, count: Option<u16>, full: bool) -> t::Line<t::GridPlacement> {
    if full {
        return t::Line {
            start: line(1),
            end: line(-1),
        };
    }
    t::Line {
        start: start
            .filter(|v| *v != 0)
            .map_or(t::GridPlacement::Auto, line),
        end: count.map_or(t::GridPlacement::Auto, |v| span(v.max(1))),
    }
}
fn style(style: &Style, layout: Layout, leaf: bool) -> t::Style {
    let options = style.layout_options.as_deref().copied().unwrap_or_default();
    let pad = style.padding_edges.unwrap_or(Insets::all(style.padding));
    let mut output = t::Style {
        display: match options.display.unwrap_or_default() {
            ViewDisplay::Grid => t::Display::Grid,
            ViewDisplay::None => t::Display::None,
            ViewDisplay::Flex => t::Display::Flex,
        },
        position: if style.absolute {
            t::Position::Absolute
        } else {
            t::Position::Relative
        },
        size: t::Size {
            width: style
                .width
                .map(Length::Px)
                .or_else(|| style.width_percent.map(Length::Percent))
                .map_or(t::Dimension::auto(), dim),
            height: style
                .height
                .map(Length::Px)
                .or_else(|| style.height_percent.map(Length::Percent))
                .map_or(t::Dimension::auto(), dim),
        },
        min_size: t::Size {
            width: options
                .min_width
                .or_else(|| style.min_width.map(Length::Px))
                .map_or(t::Dimension::length(0.), dim),
            height: options
                .min_height
                .or_else(|| style.min_height.map(Length::Px))
                .map_or(t::Dimension::length(0.), dim),
        },
        max_size: t::Size {
            width: options
                .max_width
                .or_else(|| style.max_width.map(Length::Px))
                .map_or(t::Dimension::auto(), dim),
            height: options
                .max_height
                .or_else(|| style.max_height.map(Length::Px))
                .map_or(t::Dimension::auto(), dim),
        },
        aspect_ratio: options.aspect_ratio.filter(|v| v.is_finite() && *v > 0.),
        margin: t::Rect {
            left: offset(options.margin.left.unwrap_or(Length::Px(style.margin.left))),
            right: offset(
                options
                    .margin
                    .right
                    .unwrap_or(Length::Px(style.margin.right)),
            ),
            top: offset(options.margin.top.unwrap_or(Length::Px(style.margin.top))),
            bottom: offset(
                options
                    .margin
                    .bottom
                    .unwrap_or(Length::Px(style.margin.bottom)),
            ),
        },
        inset: t::Rect {
            left: offset(options.inset.left.unwrap_or_default()),
            right: offset(options.inset.right.unwrap_or_default()),
            top: offset(options.inset.top.unwrap_or_default()),
            bottom: offset(options.inset.bottom.unwrap_or_default()),
        },
        padding: if leaf {
            taffy::style_helpers::zero()
        } else {
            t::Rect {
                left: length(options.padding.left.unwrap_or(Length::Px(pad.left))),
                right: length(options.padding.right.unwrap_or(Length::Px(pad.right))),
                top: length(options.padding.top.unwrap_or(Length::Px(pad.top))),
                bottom: length(options.padding.bottom.unwrap_or(Length::Px(pad.bottom))),
            }
        },
        gap: t::Size {
            width: length(options.gap_x.unwrap_or(Length::Px(style.gap))),
            height: length(options.gap_y.unwrap_or(Length::Px(style.gap))),
        },
        flex_direction: match (layout, options.reverse.unwrap_or(false)) {
            (Layout::Row, false) => t::FlexDirection::Row,
            (Layout::Row, true) => t::FlexDirection::RowReverse,
            (_, false) => t::FlexDirection::Column,
            (_, true) => t::FlexDirection::ColumnReverse,
        },
        flex_wrap: match options.wrap.unwrap_or_default() {
            ViewWrap::NoWrap => t::FlexWrap::NoWrap,
            ViewWrap::Wrap => t::FlexWrap::Wrap,
            ViewWrap::Reverse => t::FlexWrap::WrapReverse,
        },
        flex_grow: finite(style.flex_grow).max(0.),
        flex_shrink: finite(style.flex_shrink).max(0.),
        flex_basis: options.basis.map_or(t::Dimension::auto(), dim),
        align_items: Some(align(style.align)),
        align_self: options.align_self.map(align),
        align_content: options.align_content.map(content),
        justify_content: Some(match style.justify {
            Justify::Start => t::JustifyContent::Start,
            Justify::End => t::JustifyContent::End,
            Justify::Center => t::JustifyContent::Center,
            Justify::SpaceBetween => t::JustifyContent::SpaceBetween,
            Justify::SpaceAround => t::JustifyContent::SpaceAround,
            Justify::SpaceEvenly => t::JustifyContent::SpaceEvenly,
        }),
        grid_template_columns: options.columns.map_or_else(Vec::new, |count| {
            taffy::style_helpers::evenly_sized_tracks(count.max(1))
        }),
        grid_template_rows: options.rows.map_or_else(Vec::new, |count| {
            taffy::style_helpers::evenly_sized_tracks(count.max(1))
        }),
        grid_column: placement(
            options.column_start,
            options.column_span,
            options.column_full.unwrap_or(false),
        ),
        grid_row: placement(
            options.row_start,
            options.row_span,
            options.row_full.unwrap_or(false),
        ),
        ..Default::default()
    };
    // A leaf's natural padding is included by the retained child measurement.
    // Overlay containers retain their old overlap behavior when an opted-in
    // child introduces richer lengths; explicit Grid overrides this policy.
    if !leaf && layout == Layout::Overlay && options.display != Some(ViewDisplay::Grid) {
        output.align_items = Some(align(style.align));
    }
    output
}
fn available(value: Option<f32>) -> t::AvailableSpace {
    value.map_or(t::AvailableSpace::MaxContent, |v| {
        t::AvailableSpace::Definite(finite(v).max(0.))
    })
}
fn bound(value: t::AvailableSpace) -> Option<f32> {
    match value {
        t::AvailableSpace::Definite(v) => Some(v),
        t::AvailableSpace::MinContent => Some(0.),
        t::AvailableSpace::MaxContent => None,
    }
}
fn content_dimension(value: t::Dimension, basis: f32) -> t::Dimension {
    if value.tag() == taffy::style::CompactLength::PERCENT_TAG {
        t::Dimension::length(value.value() * basis)
    } else {
        value
    }
}
impl Scene {
    /// Resolved padding from the most recent layout, including percentages.
    pub fn padding(&self, id: NodeId) -> Insets {
        let constraints = self
            .node(id)
            .measurement
            .as_ref()
            .map_or(Constraints::default(), |m| m.constraints);
        self.resolved_padding(id, constraints)
    }
    fn resolved_padding(&self, id: NodeId, constraints: Constraints) -> Insets {
        let node = self.node(id);
        let mut padding = node
            .style
            .padding_edges
            .unwrap_or(Insets::all(node.style.padding));
        let Some(options) = node.style.layout_options.as_deref() else {
            return padding;
        };
        let basis = if node.parent.is_none() {
            Some(self.viewport.width)
        } else {
            constraints.percent_width
        };
        for (target, value) in [
            (&mut padding.left, options.padding.left),
            (&mut padding.right, options.padding.right),
            (&mut padding.top, options.padding.top),
            (&mut padding.bottom, options.padding.bottom),
        ] {
            if let Some(value) = value {
                *target = match value {
                    Length::Auto => 0.,
                    Length::Px(v) => finite(v),
                    Length::Percent(v) => percentage(Some(v), basis).unwrap_or(0.),
                }
                .max(0.);
            }
        }
        padding
    }
    pub(super) fn resolved_leaf_style(&self, id: NodeId, constraints: Constraints) -> Style {
        let mut style = self.node(id).style.clone();
        let Some(options) = style.layout_options.as_deref() else {
            return style;
        };
        let resolve = |value: Length, basis: Option<f32>| match value {
            Length::Auto => None,
            Length::Px(value) => Some(finite(value)),
            Length::Percent(value) => percentage(Some(value), basis),
        };
        for (target, value, basis) in [
            (
                &mut style.min_width,
                options.min_width,
                constraints.percent_width,
            ),
            (
                &mut style.max_width,
                options.max_width,
                constraints.percent_width,
            ),
            (
                &mut style.min_height,
                options.min_height,
                constraints.percent_height,
            ),
            (
                &mut style.max_height,
                options.max_height,
                constraints.percent_height,
            ),
        ] {
            if let Some(value) = value {
                *target = resolve(value, basis);
            }
        }
        style.padding_edges = Some(self.resolved_padding(id, constraints));
        style
    }
    pub(super) fn uses_advanced_layout(&self, id: NodeId) -> bool {
        let options = self
            .node(id)
            .style
            .layout_options
            .as_deref()
            .copied()
            .unwrap_or_default();
        let own = matches!(options.display, Some(ViewDisplay::Grid | ViewDisplay::None))
            || options.wrap.is_some()
            || options.reverse == Some(true)
            || options.gap_x.is_some()
            || options.gap_y.is_some()
            || options.align_content.is_some();
        own || self.node(id).style.align == Align::Baseline
            || self.node(id).children.iter().any(|child| {
                self.node(*child)
                    .style
                    .layout_options
                    .as_deref()
                    .is_some_and(|options| {
                        let mut options = *options;
                        options.visible = None;
                        options.clip_x = None;
                        options.clip_y = None;
                        !options.is_empty()
                    })
            })
    }
    pub(super) fn measure_hidden(&mut self, id: NodeId, mut constraints: Constraints) -> (f32, f32) {
        constraints.hidden = true;
        self.node_mut(id).advanced_layout = None;
        let ids = self.node(id).children.clone();
        let children = ids
            .into_iter()
            .map(|child| {
                self.measure(
                    child,
                    Constraints {
                        hidden: true,
                        ..Constraints::default()
                    },
                );
                (child, Rect::default())
            })
            .collect();
        self.store_measurement(id, constraints, (0., 0.), children);
        (0., 0.)
    }
    pub(super) fn measure_advanced(
        &mut self,
        id: NodeId,
        layout: Layout,
        constraints: Constraints,
    ) -> (f32, f32) {
        let root_style = self.resolved_leaf_style(id, constraints);
        let root_options = root_style
            .layout_options
            .as_deref()
            .copied()
            .unwrap_or_default();
        if root_options.display == Some(ViewDisplay::None) {
            return self.measure_hidden(id, constraints);
        }
        let mut cache = self
            .node_mut(id)
            .advanced_layout
            .take()
            .unwrap_or_else(|| Box::new(AdvancedLayoutCache::new()));
        let ids = self.node(id).children.clone();
        let live: std::collections::HashSet<_> = ids.iter().copied().collect();
        let removed: Vec<_> = cache
            .nodes
            .keys()
            .filter(|id| !live.contains(id))
            .copied()
            .collect();
        for removed in removed {
            let (key, _) = cache.nodes.remove(&removed).unwrap();
            cache.tree.remove(key).expect("owned retained layout child");
        }
        let root = cache.root;
        let mut mapping_style = root_style.clone();
        let mut mapping_options = root_options;
        mapping_options.padding = Default::default();
        mapping_options.min_width = None;
        mapping_options.max_width = None;
        mapping_options.min_height = None;
        mapping_options.max_height = None;
        mapping_style.layout_options = Some(Arc::new(mapping_options));
        let mut mapped = style(&mapping_style, layout, false);
        mapped.position = t::Position::Relative;
        mapped.margin = taffy::style_helpers::zero();
        mapped.inset = t::Rect::auto();
        mapped.size.width = constraints
            .force_width
            .or(root_style.width)
            .or_else(|| percentage(root_style.width_percent, constraints.percent_width))
            .map_or(t::Dimension::auto(), |v| dim(Length::Px(v)));
        mapped.size.height = constraints
            .force_height
            .or(root_style.height)
            .or_else(|| percentage(root_style.height_percent, constraints.percent_height))
            .map_or(t::Dimension::auto(), |v| dim(Length::Px(v)));
        // Overlay is a lumine stacking container: in-flow children contribute to
        // intrinsic size even though Taffy positions its stacked leaves absolutely.
        if layout == Layout::Overlay
            && root_options.display != Some(ViewDisplay::Grid)
            && (mapped.size.width.is_auto() || mapped.size.height.is_auto())
        {
            let padding = self.resolved_padding(id, constraints);
            let mut natural = (0_f32, 0_f32);
            for child in &ids {
                let child_style = self.node(*child).style.clone();
                if child_style.absolute {
                    continue;
                }
                let size = self.measure_intrinsic(
                    *child,
                    Constraints {
                        width: constraints.width,
                        height: constraints.height,
                        ..Default::default()
                    },
                );
                natural.0 = natural
                    .0
                    .max(size.0 + child_style.margin.left + child_style.margin.right);
                natural.1 = natural
                    .1
                    .max(size.1 + child_style.margin.top + child_style.margin.bottom);
            }
            if mapped.size.width.is_auto() {
                mapped.size.width = dim(Length::Px(natural.0 + padding.left + padding.right));
            }
            if mapped.size.height.is_auto() {
                mapped.size.height = dim(Length::Px(natural.1 + padding.top + padding.bottom));
            }
        }
        // Taffy floors negative intrinsic flex contributions, adding back a gap
        // that a clipped reveal has cancelled with its trailing margin.
        let has_negative_margin = ids.iter().any(|child| {
            let style = &self.node(*child).style;
            let options = style.layout_options.as_deref().copied().unwrap_or_default();
            let negative = |value: Option<Length>, fallback: f32| match value {
                Some(Length::Px(value) | Length::Percent(value)) => value < 0.,
                Some(Length::Auto) => false,
                None => fallback < 0.,
            };
            !style.absolute && options.display != Some(ViewDisplay::None)
                && (negative(options.margin.top, style.margin.top)
                    || negative(options.margin.bottom, style.margin.bottom))
        });
        if has_negative_margin && layout == Layout::Column && mapped.size.height.is_auto()
            && root_options.display != Some(ViewDisplay::Grid)
            && root_options.wrap.unwrap_or_default() == ViewWrap::NoWrap
            && matches!(root_options.gap_y, None | Some(Length::Px(_)))
            && !ids.iter().any(|child| {
                let style = &self.node(*child).style;
                !style.absolute && style.flex_grow > 0.
            })
        {
            let padding = self.resolved_padding(id, constraints);
            let width = constraints.force_width.or(root_style.width)
                .or_else(|| percentage(root_style.width_percent, constraints.percent_width))
                .or(constraints.width)
                .map(|w| (w - padding.left - padding.right).max(0.));
            let mut total = 0.;
            let mut count = 0usize;
            let mut cancelled = false;
            for child in &ids {
                let style = self.node(*child).style.clone();
                let options = style.layout_options.as_deref().copied().unwrap_or_default();
                if style.absolute || options.display == Some(ViewDisplay::None) { continue; }
                let px = |value: Option<Length>, fallback: f32| match value {
                    Some(Length::Px(value)) => value,
                    Some(Length::Percent(value)) => width.unwrap_or(0.) * value,
                    Some(Length::Auto) => 0.,
                    None => fallback,
                };
                let top = px(options.margin.top, style.margin.top);
                let bottom = px(options.margin.bottom, style.margin.bottom);
                let size = self.measure_intrinsic(*child, Constraints { width, percent_width: width, ..Default::default() });
                let contribution = size.1 + top + bottom;
                cancelled |= contribution < 0.;
                total += contribution;
                count += 1;
            }
            if cancelled {
                let gap = match root_options.gap_y { Some(Length::Px(value)) => value, _ => root_style.gap };
                mapped.size.height = dim(Length::Px(total + gap * count.saturating_sub(1) as f32 + padding.top + padding.bottom));
            }
        }
        let overlay_content = (layout == Layout::Overlay
            && root_options.display != Some(ViewDisplay::Grid))
        .then(|| {
            let padding = self.resolved_padding(id, constraints);
            let width = dimension(
                mapped
                    .size
                    .width
                    .into_option()
                    .expect("resolved overlay width"),
                root_style.min_width,
                root_style.max_width,
            );
            let height = dimension(
                mapped
                    .size
                    .height
                    .into_option()
                    .expect("resolved overlay height"),
                root_style.min_height,
                root_style.max_height,
            );
            (
                (width - padding.left - padding.right).max(0.),
                (height - padding.top - padding.bottom).max(0.),
            )
        });
        let children: Vec<_> = ids
            .iter()
            .map(|child| {
                let mut mapped = style(&self.node(*child).style, Layout::Column, true);
                if let Some((width, height)) = overlay_content {
                    mapped.position = t::Position::Absolute;
                    // Only in-flow layers inherit overlay alignment. Explicitly
                    // positioned layers keep their anchors in both layout paths.
                    if self.node(*child).style.absolute {
                        let padding = self.resolved_padding(id, constraints);
                        if mapped.inset.left.is_auto() && mapped.inset.right.is_auto() {
                            mapped.inset.left = t::LengthPercentageAuto::length(padding.left);
                        }
                        if mapped.inset.top.is_auto() && mapped.inset.bottom.is_auto() {
                            mapped.inset.top = t::LengthPercentageAuto::length(padding.top);
                        }
                    } else {
                        // Synthetic absolute layers retain in-flow content-box percentages.
                        // Real absolute children use Taffy's padding-box containing block.
                        for size in [&mut mapped.size, &mut mapped.min_size, &mut mapped.max_size] {
                            size.width = content_dimension(size.width, width);
                            size.height = content_dimension(size.height, height);
                        }
                    }
                }
                let revision = self.node(*child).measure_revision;
                if let Some((key, cached_revision)) = cache.nodes.get_mut(child) {
                    let key = *key;
                    if cache.tree.style(key).expect("retained style") != &mapped {
                        cache
                            .tree
                            .set_style(key, mapped)
                            .expect("retained style update");
                    }
                    if *cached_revision != revision {
                        cache
                            .tree
                            .mark_dirty(key)
                            .expect("retained child invalidation");
                        *cached_revision = revision;
                    }
                    key
                } else {
                    let key = cache
                        .tree
                        .new_leaf_with_context(mapped, *child)
                        .expect("retained leaf");
                    cache.nodes.insert(*child, (key, revision));
                    key
                }
            })
            .collect();
        if cache.order != ids {
            cache
                .tree
                .set_children(root, &children)
                .expect("retained children");
            cache.order.clone_from(&ids);
        }
        if cache.tree.style(root).expect("retained root style") != &mapped {
            cache
                .tree
                .set_style(root, mapped)
                .expect("retained root style update");
        }
        let padding = self.resolved_padding(id, constraints);
        let child_percent_width = constraints
            .force_width
            .or(root_style.width)
            .or_else(|| percentage(root_style.width_percent, constraints.percent_width))
            .map(|width| (width - padding.left - padding.right).max(0.));
        let child_percent_height = constraints
            .force_height
            .or(root_style.height)
            .or_else(|| percentage(root_style.height_percent, constraints.percent_height))
            .map(|height| (height - padding.top - padding.bottom).max(0.));
        let measure_context = (
            child_percent_width,
            child_percent_height,
            self.text_shaper_revision,
        );
        if cache.measure_context != measure_context {
            // Percentage bases and the font engine are external to Taffy's cache key.
            for child in &children {
                cache
                    .tree
                    .mark_dirty(*child)
                    .expect("retained measurement context update");
            }
            cache.measure_context = measure_context;
        }
        cache
            .tree
            .compute_layout_with_measure(
                root,
                t::Size {
                    width: available(constraints.width),
                    height: available(constraints.height),
                },
                |known, space, _, context, _| {
                    let Some(context) = context else {
                        return t::Size::ZERO;
                    };
                    let child = *context;
                    let size = self.measure_intrinsic(
                        child,
                        Constraints {
                            width: known.width.or_else(|| bound(space.width)),
                            height: known.height.or_else(|| bound(space.height)),
                            percent_width: child_percent_width,
                            percent_height: child_percent_height,
                            force_width: known.width,
                            force_height: known.height,
                            hidden: false,
                        },
                    );
                    t::Size {
                        width: size.0,
                        height: size.1,
                    }
                },
            )
            .expect("retained tree is structurally valid");
        let root_layout = *cache.tree.layout(root).expect("computed root");
        let inner_width =
            (root_layout.size.width - root_layout.padding.left - root_layout.padding.right).max(0.);
        let inner_height =
            (root_layout.size.height - root_layout.padding.top - root_layout.padding.bottom)
                .max(0.);
        let positions = ids
            .into_iter()
            .zip(children)
            .map(|(child, key)| {
                let child_layout = *cache.tree.layout(key).expect("computed child");
                let child_constraints = Constraints {
                    width: Some(child_layout.size.width),
                    height: Some(child_layout.size.height),
                    percent_width: Some(inner_width),
                    percent_height: Some(inner_height),
                    force_width: Some(child_layout.size.width),
                    force_height: Some(child_layout.size.height),
                    hidden: false,
                };
                self.measure(child, child_constraints);
                (
                    child,
                    Rect::new(
                        child_layout.location.x,
                        child_layout.location.y,
                        child_layout.size.width,
                        child_layout.size.height,
                    ),
                )
            })
            .collect();
        let size = (root_layout.size.width, root_layout.size.height);
        self.store_measurement(id, constraints, size, positions);
        self.node_mut(id).advanced_layout = Some(cache);
        size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn deep_auto_minimum_probes_preserve_finite_shallow_geometry() {
        use crate::text_layout::{FallbackTextLayout, FontStyle, WordWrap};
        fn make(depth: usize) -> (Scene, NodeId, NodeId, NodeId, Rc<Cell<usize>>) {
            let mut scene = Scene::new(400., 200.);
            let calls = Rc::new(Cell::new(0));
            let count = calls.clone();
            scene.set_font_text_measurer(move |text, size, width, font| {
                count.set(count.get() + 1);
                FallbackTextLayout::measure_with_font(text, size, width, font)
            });
            let outer = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Column),
                Style {
                    width: Some(300.),
                    ..Default::default()
                },
            );
            let mut parent = outer;
            for _ in 0..depth {
                parent = scene.append(
                    parent,
                    NodeKind::Container(Layout::Column),
                    Style {
                        width: Some(300.),
                        layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                            min_width: Some(Length::Auto),
                            gap_y: Some(Length::Px(0.)),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                );
            }
            let row = scene.append(
                parent,
                NodeKind::Container(Layout::Row),
                Style {
                    width: Some(150.),
                    ..Default::default()
                },
            );
            let text = scene.append(
                row,
                label("short longestword"),
                Style {
                    text_wrap: true,
                    flex_shrink: 1.,
                    layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                        min_width: Some(Length::Auto),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            scene.set_font(
                text,
                FontStyle {
                    word_wrap: WordWrap::Word,
                    ..Default::default()
                },
            );
            scene.append(
                row,
                NodeKind::Rect(Color(0, 0, 0, 0)),
                Style {
                    width: Some(60.),
                    height: Some(20.),
                    ..Default::default()
                },
            );
            (scene, outer, row, text, calls)
        }
        let (mut deep, outer, row, text, calls) = make(32);
        for width in [150., 260., 190., 150.] {
            let mut style = deep.style(row);
            style.width = Some(width);
            deep.set_style(row, style);
            deep.flush();
            let (mut shallow, shallow_outer, shallow_row, leaf, _) = make(0);
            let mut style = shallow.style(shallow_row);
            style.width = Some(width);
            shallow.set_style(shallow_row, style);
            shallow.flush();
            assert_eq!(
                deep.layout_bounds(outer),
                shallow.layout_bounds(shallow_outer)
            );
            assert_eq!(deep.layout_bounds(row), shallow.layout_bounds(shallow_row));
            assert_eq!(deep.layout_bounds(text), shallow.layout_bounds(leaf));
            for node in [outer, row, text] {
                let bounds = deep.layout_bounds(node);
                assert!(
                    [bounds.x, bounds.y, bounds.width, bounds.height]
                        .into_iter()
                        .all(f32::is_finite)
                );
            }
            let measured = calls.get();
            deep.flush();
            assert_eq!(
                calls.get(),
                measured,
                "unchanged layout must reuse its measurements"
            );
        }
        let font = deep.font(text);
        let longest = FallbackTextLayout::measure_with_font("longestword", 16., None, font);
        assert!((deep.layout_bounds(text).width - longest.0).abs() < 0.0001);
        assert!(deep.layout_bounds(text).height > longest.1);
    }

    #[test]
    fn min_content_keeps_word_boundaries_and_explicit_nowrap_for_plain_and_rich_text() {
        use crate::text_layout::{FallbackTextLayout, FontStyle, WordWrap};
        use std::cell::RefCell;

        let font = FontStyle {
            word_wrap: WordWrap::Word,
            ..Default::default()
        };
        let value = "short longestword";
        let rich = crate::rich_text::RichText::new(
            value,
            vec![crate::rich_text::TextRun {
                range: 0..value.len(),
                font: font.clone(),
                font_size: 16.,
                ..Default::default()
            }],
        )
        .unwrap();
        let mut scene = Scene::new(400., 200.);
        let limits = Rc::new(RefCell::new(Vec::new()));
        let plain_limits = limits.clone();
        scene.set_font_text_measurer(move |text, size, width, font| {
            plain_limits.borrow_mut().push(width);
            FallbackTextLayout::measure_with_font(text, size, width, font)
        });
        let rich_limits = limits.clone();
        scene.set_rich_text_measurer(move |rich, width| {
            rich_limits.borrow_mut().push(width);
            crate::rich_text::fallback_measure(rich, width)
        });
        assert_eq!(bound(t::AvailableSpace::MinContent), Some(0.));
        assert_eq!(bound(t::AvailableSpace::MaxContent), None);
        assert_eq!(bound(t::AvailableSpace::Definite(40.)), Some(40.));
        for kind in [
            label(value),
            NodeKind::RichText {
                text: Arc::new(rich),
            },
        ] {
            for wrap in [true, false] {
                let node = scene.append(
                    scene.root(),
                    kind.clone(),
                    Style {
                        text_wrap: wrap,
                        ..Default::default()
                    },
                );
                scene.set_font(node, font.clone());
                let mut sizes = Vec::new();
                for (space, expected) in [
                    (
                        t::AvailableSpace::MinContent,
                        if wrap { Some(0.) } else { None },
                    ),
                    (t::AvailableSpace::MaxContent, wrap.then_some(f32::INFINITY)),
                    (t::AvailableSpace::Definite(40.), wrap.then_some(40.)),
                ] {
                    limits.borrow_mut().clear();
                    sizes.push(scene.measure_intrinsic(
                        node,
                        Constraints {
                            width: bound(space),
                            ..Default::default()
                        },
                    ));
                    if wrap || sizes.len() == 1 {
                        assert_eq!(limits.borrow().as_slice(), &[expected]);
                    } else {
                        // Cached explicit-nowrap probes need no second callback.
                        assert!(limits.borrow().is_empty());
                    }
                }
                if wrap {
                    let longest =
                        FallbackTextLayout::measure_with_font("longestword", 16., None, &font);
                    assert_eq!(sizes[0].0, longest.0);
                    assert!(sizes[0].0 < sizes[1].0);
                    assert!(sizes[0].1 > sizes[1].1);
                    assert_eq!(sizes[0], sizes[2]);
                } else {
                    assert_eq!(sizes[0], sizes[1]);
                    assert_eq!(sizes[0], sizes[2]);
                }
            }
        }
    }

    #[test]
    fn automatic_flex_minimum_probes_the_longest_word_instead_of_max_content() {
        use crate::text_layout::{FallbackTextLayout, FontStyle, WordWrap};
        let mut scene = Scene::new(400., 200.);
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            ..Default::default()
        };
        let row = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Row),
            Style {
                width: Some(150.),
                ..Default::default()
            },
        );
        let text = scene.append(
            row,
            label("short longestword"),
            Style {
                text_wrap: true,
                flex_shrink: 1.,
                layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                    min_width: Some(Length::Auto),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        scene.set_font(text, font.clone());
        scene.append(
            row,
            NodeKind::Rect(Color(0, 0, 0, 0)),
            Style {
                width: Some(60.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        let longest = FallbackTextLayout::measure_with_font("longestword", 16., None, &font);
        let bounds = scene.layout_bounds(text);
        assert!(
            (bounds.width - longest.0).abs() < 0.0001,
            "{bounds:?}, longest={longest:?}"
        );
        assert!(bounds.height > longest.1, "the paragraph must still wrap");
    }

    #[test]
    fn padded_overlay_percentage_editor_allocation_settles_in_content_box() {
        let padded = Insets {
            top: 2.,
            right: 7.,
            bottom: 2.,
            left: 7.,
        };
        for (padding, height, expected_height) in [
            (padded, None, 28.),
            (padded, Some(40.), 40.),
            (Insets::all(0.), None, 24.),
        ] {
            let mut ui = crate::widgets::Ui::new(400., 300.);
            let overlay = ui.container(
                ui.root(),
                Layout::Overlay,
                Style {
                    width: Some(300.),
                    height,
                    padding_edges: Some(padding),
                    ..Default::default()
                },
            );
            let value = ui.signal("Rename".to_owned());
            let editor = ui.text_input_styled(overlay, "Rename", value.clone(), false);
            ui.scene.borrow_mut().set_style(
                editor.node,
                Style {
                    width_percent: Some(1.),
                    height_percent: Some(1.),
                    padding_edges: Some(Insets::all(0.)),
                    ..Default::default()
                },
            );
            // Like the host's positioned placeholder, this opts the Overlay
            // into advanced layout. A real absolute child keeps its padding box.
            let absolute = ui.scene.borrow_mut().append(
                overlay,
                NodeKind::Rect(Color(0, 0, 0, 0)),
                Style {
                    absolute: true,
                    width_percent: Some(1.),
                    height_percent: Some(1.),
                    layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                        inset: crate::layout::EdgeLengths {
                            left: Some(Length::Px(0.)),
                            top: Some(Length::Px(0.)),
                            ..Default::default()
                        },
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            assert!(ui.scene.borrow().uses_advanced_layout(overlay));
            ui.try_prepare_frame()
                .expect("percentage editor allocation must settle");
            assert!(ui.input.focus(&ui.scene, Some(editor.node)));
            value.set("Retained editor".to_owned());
            // Editor refresh sizes its retained viewport from its allocation.
            // Feeding padding back into the percentage height grows every pass.
            editor.refresh();
            ui.try_prepare_frame()
                .expect("retained editor allocation must stay settled");
            let scene = ui.scene.borrow();
            assert_eq!(
                scene.layout_bounds(overlay),
                Rect::new(0., 0., 300., expected_height)
            );
            assert_eq!(
                scene.layout_bounds(editor.node),
                Rect::new(
                    padding.left,
                    padding.top,
                    300. - padding.left - padding.right,
                    expected_height - padding.top - padding.bottom,
                )
            );
            assert_eq!(
                scene.layout_bounds(absolute),
                Rect::new(0., 0., 300., expected_height)
            );
        }
    }

    fn label(value: &str) -> NodeKind {
        NodeKind::Text {
            text: value.into(),
            color: Color(255, 255, 255, 255),
            font_size: 16.,
        }
    }

    fn deep_scene() -> (Scene, Vec<NodeId>, NodeId, NodeId, Rc<Cell<usize>>) {
        let mut scene = Scene::new(320., 2000.);
        let calls = Rc::new(Cell::new(0));
        let counter = calls.clone();
        // Native hosts install this font-aware measurement callback. Keep the
        // regression independent of a platform font database and GPU runtime.
        scene.set_font_text_measurer(move |text, size, width, font| {
            counter.set(counter.get() + 1);
            assert!(
                counter.get() < 4096,
                "nested layout repeatedly reshaped text"
            );
            crate::text_layout::FallbackTextLayout::measure_with_font(text, size, width, font)
        });
        let mut parent = scene.root();
        let mut containers = Vec::new();
        for depth in 0..18 {
            parent = scene.append(
                parent,
                NodeKind::Container(Layout::Column),
                Style {
                    layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                        display: Some(if depth % 2 == 0 {
                            ViewDisplay::Grid
                        } else {
                            ViewDisplay::Flex
                        }),
                        columns: Some(1),
                        wrap: Some(ViewWrap::NoWrap),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            containers.push(parent);
            scene.append(
                parent,
                label("sibling"),
                Style {
                    text_wrap: true,
                    ..Default::default()
                },
            );
        }
        let text = scene.append(
            parent,
            label("a paragraph with enough words to wrap at several widths"),
            Style {
                text_wrap: true,
                ..Default::default()
            },
        );
        let hidden = scene.append(
            parent,
            NodeKind::Container(Layout::Column),
            Style {
                layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                    display: Some(ViewDisplay::None),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        scene.append(hidden, label("hidden descendant"), Style::default());
        (scene, containers, text, hidden, calls)
    }

    #[test]
    fn deep_mixed_layout_reuses_intrinsic_constraints_without_replacing_arrange_geometry() {
        let (mut scene, containers, _, _, calls) = deep_scene();
        let outer = containers[0];
        let probes: Vec<_> = [None, Some(96.), Some(128.), Some(160.), Some(192.), Some(256.)]
            .into_iter()
            .flat_map(|width| {
                [false, true].map(|known| Constraints {
                    width,
                    percent_width: width,
                    force_width: if known { width } else { None },
                    ..Default::default()
                })
            })
            .collect();
        let sizes: Vec<_> = probes
            .iter()
            .map(|probe| scene.measure_intrinsic(outer, *probe))
            .collect();
        let first_calls = calls.get();
        assert!(first_calls > 0);
        let arranged_from = scene.node(outer).measurement.as_ref().unwrap().constraints;
        for _ in 0..3 {
            for (probe, size) in probes.iter().zip(&sizes) {
                assert_eq!(scene.measure_intrinsic(outer, *probe), *size);
            }
        }
        assert_eq!(
            calls.get(), first_calls,
            "repeated intrinsic constraints reshaped text"
        );
        assert_eq!(
            scene.node(outer).measurement.as_ref().unwrap().constraints,
            arranged_from
        );

        // A size-only hit must still restore complete child geometry at arrange.
        scene.flush();
        let (mut cold, _, _, _, _) = deep_scene();
        cold.flush();
        for (actual, expected) in scene.slots.iter().zip(&cold.slots) {
            assert_eq!(
                actual.node.as_ref().unwrap().bounds,
                expected.node.as_ref().unwrap().bounds
            );
        }
        // Revisit complete measurements after Taffy has computed another width.
        // Arrange must restore every descendant's matching snapshot on a hit.
        for width in [96., 192., 96., 256., 96., 192.] {
            let constraints = Constraints {
                width: Some(width),
                percent_width: Some(width),
                force_width: Some(width),
                ..Default::default()
            };
            scene.measure(outer, constraints);
            scene.arrange(outer, 0., 0., &mut FrameReport::default(), false);
            let (mut cold, cold_containers, _, _, _) = deep_scene();
            cold.measure(cold_containers[0], constraints);
            cold.arrange(cold_containers[0], 0., 0., &mut FrameReport::default(), false);
            for (actual, expected) in scene.slots.iter().zip(&cold.slots).skip(1) {
                assert_eq!(
                    actual.node.as_ref().unwrap().bounds,
                    expected.node.as_ref().unwrap().bounds,
                    "arrange restored the wrong constraint snapshot at width {width}"
                );
            }
        }
        assert!(
            scene.measure_calls < 50_000,
            "{} Scene measurements",
            scene.measure_calls
        );
    }

    #[test]
    fn deep_mixed_layout_invalidates_text_style_fonts_and_hidden_descendants() {
        fn change(
            scene: &mut Scene,
            containers: &[NodeId],
            text: NodeId,
            hidden: NodeId,
            phase: usize,
        ) {
            if phase >= 1 {
                scene.set_kind(text, label("short"));
            }
            if phase >= 2 {
                let id = *containers.last().unwrap();
                let mut style = scene.style(id);
                style.padding = 7.;
                scene.set_style(id, style);
            }
            if phase >= 3 {
                scene.set_font(
                    text,
                    crate::text_layout::FontStyle {
                        letter_spacing: crate::text_layout::LetterSpacing::px(2.),
                        ..Default::default()
                    },
                );
                let descendant = scene.node(hidden).children[0];
                scene.set_kind(descendant, label("replacement while hidden"));
            }
            if phase >= 4 {
                let mut style = scene.style(hidden);
                style.layout_options = None;
                scene.set_style(hidden, style);
            }
        }
        let (mut scene, containers, text, hidden, calls) = deep_scene();
        for phase in 0..=4 {
            change(&mut scene, &containers, text, hidden, phase);
            scene.flush();
            let (mut cold, cold_containers, cold_text, cold_hidden, _) = deep_scene();
            change(&mut cold, &cold_containers, cold_text, cold_hidden, phase);
            cold.flush();
            for (actual, expected) in scene.slots.iter().zip(&cold.slots) {
                assert_eq!(
                    actual.node.as_ref().unwrap().bounds,
                    expected.node.as_ref().unwrap().bounds,
                    "warm and cold geometry differ after change {phase}"
                );
            }
        }
        assert!(calls.get() > 0);
        assert!(
            scene.measure_calls < 50_000,
            "{} Scene measurements",
            scene.measure_calls
        );
    }

    #[test]
    fn intrinsic_probe_cache_is_bounded_and_invalidates_before_arrange() {
        let mut scene = Scene::new(320., 200.);
        let text = scene.append(scene.root(), label("original"), Style::default());
        for width in 0..INTRINSIC_MEASUREMENTS + 16 {
            scene.measure_intrinsic(
                text,
                Constraints {
                    width: Some(width as f32),
                    ..Default::default()
                },
            );
        }
        assert!(scene.node(text).intrinsic_sizes.len() <= INTRINSIC_MEASUREMENTS);
        let constraints = Constraints::default();
        let before = scene.measure_intrinsic(text, constraints);
        scene.set_kind(text, label("a much longer replacement"));
        assert!(scene.measure_intrinsic(text, constraints).0 > before.0);
        scene.set_font_text_measurer(|_, _, _, _| (5., 7.));
        assert_eq!(scene.measure_intrinsic(text, constraints), (5., 7.));
    }

    #[test]
    fn advanced_cache_survives_changes_and_removes_stale_children() {
        let mut scene = Scene::new(500., 300.);
        let root = scene.root();
        let grid = scene.append(
            root,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(300.),
                height: Some(200.),
                layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                    display: Some(ViewDisplay::Grid),
                    columns: Some(2),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let first = scene.append(
            grid,
            NodeKind::Container(Layout::Column),
            Style {
                height: Some(20.),
                ..Default::default()
            },
        );
        let second = scene.append(
            grid,
            NodeKind::Container(Layout::Column),
            Style {
                height: Some(30.),
                ..Default::default()
            },
        );
        scene.flush();
        let cache = scene.node(grid).advanced_layout.as_ref().unwrap();
        let pointer = (&**cache) as *const AdvancedLayoutCache;
        let second_key = cache.nodes[&second];
        assert!(scene.node(first).advanced_layout.is_none());
        assert_eq!(scene.flush().layout_nodes, 0);
        let mut style = scene.style(first);
        style.height = Some(40.);
        scene.set_style(first, style);
        scene.flush();
        assert_eq!(
            (&**scene.node(grid).advanced_layout.as_ref().unwrap()) as *const AdvancedLayoutCache,
            pointer
        );
        assert_eq!(
            scene.node(grid).advanced_layout.as_ref().unwrap().nodes[&second],
            second_key
        );
        scene.remove(first);
        scene.flush();
        let cache = scene.node(grid).advanced_layout.as_ref().unwrap();
        assert!(!cache.nodes.contains_key(&first));
        assert_eq!(cache.nodes.len(), 1);
        assert_eq!(cache.nodes[&second], second_key);
    }
}
