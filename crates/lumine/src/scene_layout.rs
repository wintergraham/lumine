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
    /// Only discard ancestor inputs that this node's own measurement cannot
    /// consume. Keep the original query separately for arrangement and padding.
    pub(super) fn measurement_key(&self, id: NodeId, mut query: Constraints) -> Constraints {
        let style = &self.node(id).style;
        let options = style.layout_options.as_deref();
        query.hidden |= options.is_some_and(|options| options.display == Some(ViewDisplay::None));
        let definite_width = query.force_width.is_some()
            || (!query.min_content_width && style.width.is_some());
        let definite_height = query.force_height.is_some() || style.height.is_some();
        if definite_width {
            query.width = None;
        }
        if definite_height {
            query.height = None;
        }
        let percent = |value| matches!(value, Some(Length::Percent(_)));
        // Percentage padding on either axis uses the parent's width. Min/max
        // percentages remain relevant even when the preferred size is fixed.
        let width_basis = (!definite_width && style.width_percent.is_some())
            || options.is_some_and(|options| {
                percent(options.min_width) || percent(options.max_width)
                    || percent(options.padding.left) || percent(options.padding.right)
                    || percent(options.padding.top) || percent(options.padding.bottom)
            });
        let height_basis = (!definite_height && style.height_percent.is_some())
            || options.is_some_and(|options| percent(options.min_height) || percent(options.max_height));
        if !width_basis {
            query.percent_width = None;
        }
        if !height_basis {
            query.percent_height = None;
        }
        query
    }

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
        if constraints.min_content_width && constraints.force_width.is_none() {
            // Taffy asks for this item's content contribution, not its preferred
            // width. Descendant styles remain intact for their own contributions.
            style.width = None;
            style.width_percent = None;
        }
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
    pub(super) fn child_min_content_width(
        &self,
        child: NodeId,
        constraints: Constraints,
        percent_width: Option<f32>,
    ) -> bool {
        let style = &self.node(child).style;
        // Auto descendants continue the content query. A descendant's own
        // definite preference still contributes, even when the queried root's does not.
        constraints.min_content_width
            && constraints.force_width.is_none()
            && style.width.is_none()
            && percentage(style.width_percent, percent_width).is_none()
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
        let mut synthesized_height = false;
        // Overlay is a lumine stacking container: in-flow children contribute to
        // intrinsic size even though Taffy positions its stacked leaves absolutely.
        if layout == Layout::Overlay
            && root_options.display != Some(ViewDisplay::Grid)
            && (mapped.size.width.is_auto() || mapped.size.height.is_auto())
        {
            let padding = self.resolved_padding(id, constraints);
            let known_width = constraints.force_width.or(root_style.width)
                .or_else(|| percentage(root_style.width_percent, constraints.percent_width))
                .map(|width| dimension(width, root_style.min_width, root_style.max_width));
            let known_height = constraints.force_height.or(root_style.height)
                .or_else(|| percentage(root_style.height_percent, constraints.percent_height))
                .map(|height| dimension(height, root_style.min_height, root_style.max_height));
            let inner_width = known_width.map(|width| (width - padding.left - padding.right).max(0.));
            let inner_height = known_height.map(|height| (height - padding.top - padding.bottom).max(0.));
            let available_width = known_width.or_else(|| constraints.width.map(|width| width.min(root_style.max_width.unwrap_or(f32::INFINITY))))
                .or(root_style.max_width).map(|width| (width - padding.left - padding.right).max(0.));
            let available_height = known_height.or_else(|| constraints.height.map(|height| height.min(root_style.max_height.unwrap_or(f32::INFINITY))))
                .or(root_style.max_height).map(|height| (height - padding.top - padding.bottom).max(0.));
            let mut measure_content = |width, percent_width, min_content_width| {
                let mut natural = (0_f32, 0_f32);
                for child in &ids {
                    let child_style = self.node(*child).style.clone();
                    if child_style.absolute {
                        continue;
                    }
                    let size = self.measure_intrinsic(
                        *child,
                        Constraints {
                            // Wrapped intrinsic height uses this overlay's content box,
                            // rather than its ancestor's wider available allocation.
                            width,
                            height: available_height,
                            percent_width,
                            percent_height: inner_height,
                            min_content_width: min_content_width
                                && self.child_min_content_width(*child, constraints, percent_width),
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
                natural
            };
            let mut natural =
                measure_content(available_width, inner_width, constraints.min_content_width);
            let auto_width = mapped.size.width.is_auto();
            if auto_width {
                mapped.size.width = dim(Length::Px(natural.0 + padding.left + padding.right));
            }
            if auto_width && mapped.size.height.is_auto() {
                let width = dimension(
                    mapped
                        .size
                        .width
                        .into_option()
                        .expect("resolved overlay width"),
                    root_style.min_width,
                    root_style.max_width,
                );
                let content_width = (width - padding.left - padding.right).max(0.);
                // Intrinsic width is selected before percentage bounds can resolve.
                // Reflow height at that width while auto percentage height stays indefinite.
                natural = measure_content(Some(content_width), Some(content_width), false);
            }
            if mapped.size.height.is_auto() {
                mapped.size.height = dim(Length::Px(natural.1 + padding.top + padding.bottom));
                synthesized_height = true;
            }
        }
        // Taffy floors negative intrinsic flex contributions, adding back a gap
        // that a clipped reveal has cancelled with its trailing margin. Its
        // MaxContent flex fraction also scales negative margins by the basis,
        // collapsing even positive outer contributions in an auto-height column.
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
                .map(|w| dimension(w, root_style.min_width, root_style.max_width))
                .or_else(|| constraints.width.map(|w| w.min(root_style.max_width.unwrap_or(f32::INFINITY))))
                .or(root_style.max_width)
                .map(|w| (w - padding.left - padding.right).max(0.));
            let mut total = 0.;
            let mut count = 0usize;
            let mut cancelled = false;
            let mut auto_basis = true;
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
                let size = self.measure_intrinsic(
                    *child,
                    Constraints {
                        width,
                        percent_width: width,
                        min_content_width: self.child_min_content_width(*child, constraints, width),
                        ..Default::default()
                    },
                );
                let contribution = size.1 + top + bottom;
                cancelled |= contribution < 0.;
                auto_basis &= matches!(options.basis, None | Some(Length::Auto | Length::Percent(_)));
                total += contribution;
                count += 1;
            }
            // Authored bases may differ from content size; leave their
            // MaxContent sizing to Taffy instead of replacing it with this sum.
            if cancelled || (constraints.height.is_none() && auto_basis) {
                let gap = match root_options.gap_y { Some(Length::Px(value)) => value, _ => root_style.gap };
                mapped.size.height = dim(Length::Px(total + gap * count.saturating_sub(1) as f32 + padding.top + padding.bottom));
                synthesized_height = true;
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
                if synthesized_height && !self.node(*child).style.absolute {
                    // An intrinsic height gives Taffy an allocation, not a definite
                    // percentage-height basis. Keep the original auto-height
                    // contract, matching child_percent_height in the callback.
                    let unresolved = |value: &mut t::Dimension, fallback| {
                        if value.tag() == taffy::style::CompactLength::PERCENT_TAG {
                            *value = fallback;
                        }
                    };
                    unresolved(&mut mapped.size.height, t::Dimension::auto());
                    unresolved(&mut mapped.min_size.height, t::Dimension::length(0.));
                    unresolved(&mut mapped.max_size.height, t::Dimension::auto());
                    if layout == Layout::Column {
                        unresolved(&mut mapped.flex_basis, t::Dimension::auto());
                    }
                }
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
        // Match the definite content box used by the overlay intrinsic prepass
        // and final arrangement, including min/max clamps on preferred sizes.
        let child_percent_width = overlay_content.map(|(width, _)| width).or_else(|| {
            constraints
                .force_width
                .or(root_style.width)
                .or_else(|| percentage(root_style.width_percent, constraints.percent_width))
                .map(|width| dimension(width, root_style.min_width, root_style.max_width))
                .map(|width| (width - padding.left - padding.right).max(0.))
        });
        let child_percent_height = constraints
            .force_height
            .or(root_style.height)
            .or_else(|| percentage(root_style.height_percent, constraints.percent_height))
            .map(|height| dimension(height, root_style.min_height, root_style.max_height))
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
        let intrinsic_column_width = constraints.min_content_width
            && constraints.force_width.is_none()
            && cache
                .tree
                .style(root)
                .expect("retained root style")
                .size
                .width
                .is_auto()
            && layout == Layout::Column
            && root_options.display != Some(ViewDisplay::Grid);
        let available_space = t::Size {
            width: if constraints.min_content_width && constraints.force_width.is_none() {
                t::AvailableSpace::MinContent
            } else {
                available(constraints.width)
            },
            height: available(constraints.height),
        };
        let mut measure = |known: t::Size<Option<f32>>,
                           space: t::Size<t::AvailableSpace>,
                           _: t::NodeId,
                           context: Option<&mut NodeId>,
                           _: &t::Style| {
            let Some(context) = context else {
                return t::Size::ZERO;
            };
            let child = *context;
            let query = Constraints {
                width: known.width.or_else(|| bound(space.width)),
                min_content_width: known.width.is_none()
                    && matches!(space.width, t::AvailableSpace::MinContent),
                height: known.height.or_else(|| bound(space.height)),
                percent_width: child_percent_width,
                percent_height: child_percent_height,
                force_width: known.width,
                force_height: known.height,
                hidden: false,
            };
            let size = self.measure_intrinsic(child, query);
            self.trace_layout(
                child,
                match space.height {
                    t::AvailableSpace::MinContent => "taffy.height.min-content",
                    t::AvailableSpace::MaxContent => "taffy.height.max-content",
                    t::AvailableSpace::Definite(_) => "taffy.height.definite",
                },
                Some(query),
                Some(size),
            );
            t::Size {
                width: size.0,
                height: size.1,
            }
        };
        cache
            .tree
            .compute_layout_with_measure(root, available_space, &mut measure)
            .expect("retained tree is structurally valid");
        if intrinsic_column_width {
            // Select intrinsic width before reflowing cross-axis-dependent height.
            // Keep the callback's original percentage bases, especially auto height.
            let width = cache
                .tree
                .layout(root)
                .expect("computed intrinsic width")
                .size
                .width;
            let mut resolved = cache.tree.style(root).expect("retained root style").clone();
            resolved.size.width = dim(Length::Px(width));
            cache
                .tree
                .set_style(root, resolved)
                .expect("resolved intrinsic width");
            // A MinContent result can have this width while retaining height
            // from a narrower probe. Reflow children under the selected width.
            for child in &children {
                cache
                    .tree
                    .mark_dirty(*child)
                    .expect("intrinsic width reflow");
            }
            cache
                .tree
                .compute_layout_with_measure(root, available_space, &mut measure)
                .expect("retained tree is structurally valid");
        }
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
                    percent_height: if synthesized_height && !self.node(child).style.absolute {
                        child_percent_height
                    } else {
                        Some(inner_height)
                    },
                    force_width: Some(child_layout.size.width),
                    force_height: Some(child_layout.size.height),
                    min_content_width: false,
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
    fn empty_preferred_width_auto_minimum_shrinks_to_the_actual_flex_allocation() {
        let mut scene = Scene::new(400., 200.);
        let row = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Row),
            Style {
                width: Some(226.),
                gap: 10.,
                ..Default::default()
            },
        );
        scene.append(
            row,
            NodeKind::Rect(Color(0, 0, 0, 0)),
            Style {
                width: Some(20.),
                height: Some(13.),
                ..Default::default()
            },
        );
        let child = scene.append(
            row,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(196.097),
                height: Some(13.),
                flex_shrink: 1.,
                layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                    min_width: Some(Length::Auto),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        scene.flush();
        let bounds = scene.layout_bounds(child);
        assert!((bounds.width - 196.).abs() < 0.0001, "{bounds:?}");
        assert!((bounds.x - 30.).abs() < 0.0001, "{bounds:?}");
        assert_eq!(bounds.height, 13.);
    }

    #[test]
    fn min_content_query_preserves_descendant_preferences_known_zero_and_other_constraints() {
        use crate::text_layout::{FallbackTextLayout, FontStyle, WordWrap};
        use std::cell::RefCell;
        let min = Constraints {
            width: Some(0.),
            min_content_width: true,
            ..Default::default()
        };
        let definite = Constraints {
            width: Some(0.),
            ..Default::default()
        };
        let known_zero = Constraints {
            force_width: Some(0.),
            ..min
        };
        let mut scene = Scene::new(400., 200.);
        let empty = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(196.097),
                ..Default::default()
            },
        );
        // Identical numeric available width, different query identity and result.
        assert_eq!(scene.measure_intrinsic(empty, min).0, 0.);
        assert_eq!(scene.measure_intrinsic(empty, definite).0, 196.097);
        assert_eq!(scene.measure_intrinsic(empty, known_zero).0, 0.);
        assert_eq!(
            scene.measure_intrinsic(empty, Constraints::default()).0,
            196.097
        );
        assert_eq!(scene.measure_intrinsic(empty, min).0, 0.);
        let mut style = scene.style(empty);
        style.min_width = Some(30.);
        style.max_width = Some(80.);
        scene.set_style(empty, style);
        assert_eq!(scene.measure_intrinsic(empty, min).0, 30.);
        assert_eq!(
            scene.measure_intrinsic(empty, Constraints::default()).0,
            80.
        );
        let percentage = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                width_percent: Some(0.5),
                ..Default::default()
            },
        );
        let percent = Constraints {
            percent_width: Some(200.),
            ..Default::default()
        };
        assert_eq!(scene.measure_intrinsic(percentage, percent).0, 100.);
        assert_eq!(
            scene
                .measure_intrinsic(
                    percentage,
                    Constraints {
                        percent_width: percent.percent_width,
                        ..min
                    }
                )
                .0,
            0.
        );
        assert_eq!(
            scene
                .measure_intrinsic(
                    percentage,
                    Constraints {
                        percent_width: percent.percent_width,
                        ..known_zero
                    }
                )
                .0,
            0.
        );

        for layout in [Layout::Column, Layout::Row, Layout::Overlay] {
            for advanced in [false, true] {
                for preferred in [None, Some(196.097)] {
                    let wrapper = scene.append(
                        scene.root(),
                        NodeKind::Container(layout),
                        Style {
                            width: preferred,
                            layout_options: advanced.then(|| {
                                Arc::new(crate::layout::LayoutOptions {
                                    gap_y: Some(Length::Px(0.)),
                                    ..Default::default()
                                })
                            }),
                            ..Default::default()
                        },
                    );
                    scene.append(
                        wrapper,
                        NodeKind::Container(Layout::Column),
                        Style {
                            width: Some(120.),
                            height: Some(13.),
                            ..Default::default()
                        },
                    );
                    assert_eq!(
                        scene.measure_intrinsic(wrapper, min).0,
                        120.,
                        "{layout:?}, advanced={advanced}, preferred={preferred:?}"
                    );
                }
            }
        }

        let limits = Rc::new(RefCell::new(Vec::new()));
        let measured = limits.clone();
        scene.set_font_text_measurer(move |text, size, width, font| {
            measured.borrow_mut().push(width);
            FallbackTextLayout::measure_with_font(text, size, width, font)
        });
        for wrap in [true, false] {
            let text = scene.append(
                scene.root(),
                label("short longestword"),
                Style {
                    width: Some(196.097),
                    text_wrap: wrap,
                    ..Default::default()
                },
            );
            let font = FontStyle {
                word_wrap: WordWrap::Word,
                ..Default::default()
            };
            scene.set_font(text, font.clone());
            limits.borrow_mut().clear();
            let size = scene.measure_intrinsic(text, min);
            assert_eq!(limits.borrow().as_slice(), &[wrap.then_some(0.)]);
            let expected = FallbackTextLayout::measure_with_font(
                if wrap {
                    "longestword"
                } else {
                    "short longestword"
                },
                16.,
                None,
                &font,
            );
            assert_eq!(size.0, expected.0);
            assert_eq!(scene.measure_intrinsic(text, known_zero).0, 0.);
            assert_eq!(scene.measure_intrinsic(text, definite).0, 196.097);
            assert_eq!(
                scene.measure_intrinsic(text, Constraints::default()).0,
                196.097
            );
        }
    }

    #[test]
    fn min_content_survives_auto_containers_without_erasing_fixed_descendants() {
        use crate::text_layout::{FallbackTextLayout, FontStyle, WordWrap};
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            ..Default::default()
        };
        let longest = FallbackTextLayout::measure_with_font("longestword", 16., None, &font).0;
        let min = Constraints {
            width: Some(0.),
            min_content_width: true,
            ..Default::default()
        };
        for layout in [Layout::Column, Layout::Row, Layout::Overlay] {
            for fixed in [false, true] {
                let mut scene = Scene::new(400., 200.);
                // The ordinary outer container must preserve the typed probe
                // until the advanced auto descendant reaches its text callback.
                let outer = scene.append(
                    scene.root(),
                    NodeKind::Container(Layout::Column),
                    Style {
                        width: Some(196.097),
                        ..Default::default()
                    },
                );
                let auto = scene.append(
                    outer,
                    NodeKind::Container(layout),
                    Style {
                        layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                            gap_y: Some(Length::Px(0.)),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                );
                if fixed {
                    // A further ordinary auto wrapper must retain the fixed
                    // grandchild's contribution through the same query path.
                    let wrapper =
                        scene.append(auto, NodeKind::Container(Layout::Column), Style::default());
                    scene.append(
                        wrapper,
                        NodeKind::Container(Layout::Column),
                        Style {
                            width: Some(120.),
                            height: Some(13.),
                            ..Default::default()
                        },
                    );
                } else {
                    let text = scene.append(
                        auto,
                        label("short longestword"),
                        Style {
                            text_wrap: true,
                            layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                                min_width: Some(Length::Auto),
                                ..Default::default()
                            })),
                            ..Default::default()
                        },
                    );
                    scene.set_font(text, font.clone());
                }
                let expected = if fixed { 120. } else { longest };
                assert_eq!(
                    scene.measure_intrinsic(outer, min).0,
                    expected,
                    "{layout:?}, fixed={fixed}"
                );
                assert_eq!(
                    scene
                        .measure_intrinsic(
                            outer,
                            Constraints {
                                width: Some(0.),
                                ..Default::default()
                            }
                        )
                        .0,
                    196.097
                );
                assert_eq!(
                    scene
                        .measure_intrinsic(
                            outer,
                            Constraints {
                                force_width: Some(0.),
                                ..min
                            }
                        )
                        .0,
                    0.
                );
                assert_eq!(
                    scene.measure_intrinsic(outer, Constraints::default()).0,
                    196.097
                );
                assert_eq!(
                    scene.measure_intrinsic(outer, min).0,
                    expected,
                    "cached {layout:?}, fixed={fixed}"
                );
            }
        }
    }

    #[test]
    fn height_only_flex_allocations_keep_the_min_content_width_probe() {
        use crate::text_layout::{FallbackTextLayout, FontStyle, LineHeight, WordWrap};
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            line_height: LineHeight::px(10.),
            ..Default::default()
        };
        let longest = FallbackTextLayout::measure_with_font("longestword", 16., None, &font).0;
        let min = Constraints {
            width: Some(0.),
            min_content_width: true,
            ..Default::default()
        };
        // 20 needs only the definite main-allocation pass; 60 also grows the
        // child in the redistribution loop. Neither allocates a definite width.
        for height in [20., 60.] {
            for preferred in [None, Some(120.)] {
                let mut scene = Scene::new(400., 200.);
                let outer = scene.append(
                    scene.root(),
                    NodeKind::Container(Layout::Column),
                    Style {
                        width: Some(196.097),
                        height: Some(height),
                        ..Default::default()
                    },
                );
                let inner = scene.append(
                    outer,
                    NodeKind::Container(Layout::Column),
                    Style {
                        width: preferred,
                        flex_grow: 1.,
                        align: Align::Stretch,
                        ..Default::default()
                    },
                );
                let text = scene.append(
                    inner,
                    label("short longestword"),
                    Style {
                        text_wrap: true,
                        layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                            min_width: Some(Length::Auto),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                );
                scene.set_font(text, font.clone());
                assert!(!scene.uses_advanced_layout(outer));
                assert!(scene.uses_advanced_layout(inner));
                let expected = (preferred.unwrap_or(longest), height);
                assert_eq!(
                    scene.measure(outer, min),
                    expected,
                    "height={height}, preferred={preferred:?}"
                );
                let allocation = scene.node(inner).measurement.as_ref().unwrap().constraints;
                assert_eq!(allocation.force_height, Some(height));
                assert_eq!(allocation.force_width, None);
                assert_eq!(allocation.min_content_width, preferred.is_none());
                assert_eq!(
                    scene
                        .measure(
                            outer,
                            Constraints {
                                force_width: Some(0.),
                                ..min
                            }
                        )
                        .0,
                    0.
                );
                assert_eq!(scene.measure(outer, Constraints::default()).0, 196.097);
                assert_eq!(scene.measure(outer, min), expected);
            }
        }
    }

    #[test]
    fn centered_column_overlay_reflows_wrapping_row_before_synthesizing_height() {
        use crate::layout::LayoutOptions;
        use crate::text_layout::{FallbackTextLayout, FontStyle, LineHeight, WordWrap};

        let value = "Describe what to watch, when to act, and what you want your automation to do. Review its plan before turning it on.";
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            line_height: LineHeight::px(18.75),
            ..Default::default()
        };
        for wrap in [true, false] {
            for authored_height in [None, Some(90.)] {
                let mut scene = Scene::new(500., 400.);
                scene.set_font_text_measurer(|text, size, width, font| {
                    FallbackTextLayout::measure_with_font(text, size, width, font)
                });
                let column = scene.append(
                    scene.root(),
                    NodeKind::Container(Layout::Column),
                    Style {
                        width: Some(385.),
                        height: Some(300.),
                        padding_edges: Some(Insets {
                            left: 80.,
                            right: 80.,
                            ..Default::default()
                        }),
                        align: Align::Center,
                        justify: Justify::Center,
                        gap: 10.,
                        layout_options: Some(Arc::new(LayoutOptions {
                            wrap: Some(ViewWrap::NoWrap),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                );
                let icon = scene.append(
                    column,
                    NodeKind::Rect(Color(0, 0, 0, 0)),
                    Style {
                        width: Some(18.),
                        height: Some(18.),
                        ..Default::default()
                    },
                );
                let overlay = scene.append(
                    column,
                    NodeKind::Container(Layout::Overlay),
                    Style {
                        height: authored_height,
                        flex_shrink: 1.,
                        align: Align::Stretch,
                        layout_options: Some(Arc::new(LayoutOptions {
                            max_width: Some(Length::Percent(1.)),
                            min_height: Some(Length::Auto),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                );
                let row = scene.append(
                    overlay,
                    NodeKind::Container(Layout::Row),
                    Style {
                        height_percent: Some(1.),
                        min_width: Some(0.),
                        align: Align::Baseline,
                        layout_options: Some(Arc::new(LayoutOptions {
                            wrap: Some(ViewWrap::Wrap),
                            min_width: Some(Length::Percent(1.)),
                            max_width: Some(Length::Percent(1.)),
                            ..Default::default()
                        })),
                        ..Default::default()
                    },
                );
                let wrapper = scene.append(
                    row,
                    NodeKind::Container(Layout::Column),
                    Style {
                        flex_shrink: 1.,
                        text_wrap: wrap,
                        ..Default::default()
                    },
                );
                let text = scene.append(
                    wrapper,
                    NodeKind::Text {
                        text: value.into(),
                        font_size: 12.5,
                        color: Color(0, 0, 0, 255),
                    },
                    Style {
                        text_wrap: wrap,
                        ..Default::default()
                    },
                );
                scene.set_font(text, font.clone());
                let minimum = scene.measure_intrinsic(
                    overlay,
                    Constraints {
                        width: Some(0.),
                        min_content_width: true,
                        ..Default::default()
                    },
                );
                let minimum_width =
                    FallbackTextLayout::measure_with_font(value, 12.5, wrap.then_some(0.), &font).0;
                assert!(
                    (minimum.0 - minimum_width).abs() < 0.001,
                    "{minimum:?}, expected minimum {minimum_width}"
                );
                for width in [385., 435., 385.] {
                    let content_width = width - 160.;
                    let expected_text = FallbackTextLayout::measure_with_font(
                        value,
                        12.5,
                        wrap.then_some(content_width),
                        &font,
                    );
                    let expected_height = authored_height.unwrap_or(expected_text.1);
                    let intrinsic = scene.measure(
                        overlay,
                        Constraints {
                            width: Some(content_width),
                            percent_width: Some(content_width),
                            ..Default::default()
                        },
                    );
                    assert!(
                        (intrinsic.0 - content_width).abs() < 0.001
                            && (intrinsic.1 - expected_height).abs() < 0.001,
                        "wrap={wrap}, authored={authored_height:?}: {intrinsic:?}, expected {content_width}x{expected_height}"
                    );
                    assert_eq!(
                        scene
                            .node(row)
                            .measurement
                            .as_ref()
                            .unwrap()
                            .constraints
                            .percent_height,
                        authored_height
                    );
                    let mut style = scene.style(column);
                    style.width = Some(width);
                    scene.set_style(column, style);
                    let size = scene.measure(column, Constraints::default());
                    assert_eq!(size, (width, 300.));
                    scene.arrange(column, 0., 0., &mut FrameReport::default(), false);
                    let bounds = scene.layout_bounds(overlay);
                    let row_bounds = scene.layout_bounds(row);
                    assert!(
                        (bounds.width - content_width).abs() < 0.001
                            && (bounds.height - expected_height).abs() < 0.001,
                        "overlay {bounds:?}"
                    );
                    assert!(
                        (row_bounds.height - expected_height).abs() < 0.001,
                        "row {row_bounds:?}"
                    );
                    assert!((scene.layout_bounds(wrapper).height - expected_text.1).abs() < 0.001);
                    assert!((scene.layout_bounds(text).height - expected_text.1).abs() < 0.001);
                    let expected_icon_y = (300. - 18. - 10. - expected_height) / 2.;
                    assert!((scene.layout_bounds(icon).y - expected_icon_y).abs() < 0.001);
                    assert!((bounds.y - expected_icon_y - 28.).abs() < 0.001);
                    assert!((bounds.x - 80.).abs() < 0.001);
                }
            }
        }
    }

    #[test]
    fn clamped_overlay_percent_padding_matches_intrinsic_and_final_text_height() {
        use crate::{
            layout::EdgeLengths,
            text_layout::{FallbackTextLayout, FontStyle, LineHeight},
        };
        use std::cell::RefCell;
        let mut scene = Scene::new(400., 200.);
        let limits = Rc::new(RefCell::new(Vec::new()));
        let measured = limits.clone();
        scene.set_font_text_measurer(move |text, size, width, font| {
            measured.borrow_mut().push(width);
            FallbackTextLayout::measure_with_font(text, size, width, font)
        });
        let overlay = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(120.),
                max_width: Some(60.),
                ..Default::default()
            },
        );
        let text = scene.append(
            overlay,
            NodeKind::Text {
                text: "abcdefg".into(),
                font_size: 10.,
                color: Color(255, 255, 255, 255),
            },
            Style {
                text_wrap: true,
                layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                    padding: EdgeLengths {
                        left: Some(Length::Percent(0.1)),
                        right: Some(Length::Percent(0.1)),
                        ..Default::default()
                    },
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        scene.set_font(
            text,
            FontStyle {
                line_height: LineHeight::px(20.),
                ..Default::default()
            },
        );
        // Changing only the owner's clamp must also change the retained
        // callback percentage context, even though its preferred width stays 120.
        for cap in [60., 80., 60.] {
            let mut style = scene.style(overlay);
            style.max_width = Some(cap);
            scene.set_style(overlay, style);
            limits.borrow_mut().clear();
            scene.flush();
            assert_eq!(scene.layout_bounds(overlay), Rect::new(0., 0., cap, 20.));
            let padding = scene.padding(text);
            assert_eq!((padding.left, padding.right), (cap * 0.1, cap * 0.1));
            assert_eq!(scene.layout_bounds(text).height, 20.);
            assert_eq!(
                scene.layout_bounds(text).width,
                42. + padding.left + padding.right
            );
            assert!(
                !limits.borrow().contains(&Some(36.)),
                "unclamped preferred width reached text: {:?}",
                limits.borrow()
            );
            let calls = limits.borrow().len();
            scene.flush();
            assert_eq!(
                limits.borrow().len(),
                calls,
                "unchanged layout must reuse its measurements"
            );
        }
    }

    #[test]
    fn stroked_negative_margin_column_keeps_max_content_height_through_native_ancestors() {
        use crate::layout::{EdgeLengths, LayoutOptions};
        let mut scene = Scene::new(800., 600.);
        let stage = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(800.),
                height: Some(400.),
                align: Align::Center,
                justify: Justify::End,
                ..Default::default()
            },
        );
        let wrapper = scene.append(stage, NodeKind::Container(Layout::Column), Style::default());
        let overlay = scene.append(
            wrapper,
            NodeKind::Container(Layout::Overlay),
            Style::default(),
        );
        let owner = scene.append(
            overlay,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(594.),
                gap: 15.,
                ..Default::default()
            },
        );
        // The hidden sibling selects advanced layout for the retained owner,
        // while the surrounding wrappers and composer remain ordinary columns.
        scene.append(
            owner,
            NodeKind::Container(Layout::Column),
            Style {
                layout_options: Some(Arc::new(LayoutOptions {
                    display: Some(ViewDisplay::None),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let composer = scene.append(
            owner,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(594.),
                gap: 9.,
                ..Default::default()
            },
        );
        let fill = Style {
            width_percent: Some(1.),
            ..Default::default()
        };
        let body = scene.append(composer, NodeKind::Container(Layout::Column), fill.clone());
        let input = scene.append(
            body,
            NodeKind::Container(Layout::Column),
            Style {
                padding: 9.,
                gap: 12.,
                ..fill.clone()
            },
        );
        let stroke = scene.append(
            input,
            NodeKind::Container(Layout::Overlay),
            Style {
                absolute: true,
                layout_options: Some(Arc::new(LayoutOptions {
                    inset: EdgeLengths::all(Length::Px(-0.75)),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        // The viewport includes 33px of padding and cancels 30px vertically.
        // Its outer contribution stays positive, which the old cancelled-only
        // intrinsic correction missed under MaxContent.
        let attachments = scene.append(
            input,
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(588.),
                height: Some(93.),
                padding_edges: Some(Insets {
                    top: 18.,
                    right: 3.,
                    bottom: 15.,
                    left: 15.,
                }),
                margin: Insets {
                    top: -15.,
                    bottom: -15.,
                    left: -12.,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let tile = scene.append(
            attachments,
            NodeKind::Rect(Color(34, 51, 68, 255)),
            Style {
                width: Some(570.),
                height: Some(60.),
                ..Default::default()
            },
        );
        let editor = scene.append(input, NodeKind::Container(Layout::Row), fill.clone());
        scene.append(
            editor,
            NodeKind::Rect(Color(255, 255, 255, 255)),
            Style {
                width: Some(24.),
                height: Some(48.),
                ..Default::default()
            },
        );
        let footer = scene.append(
            composer,
            NodeKind::Container(Layout::Row),
            Style {
                height: Some(39.),
                ..fill
            },
        );
        assert!(scene.uses_advanced_layout(owner));
        assert!(scene.uses_advanced_layout(input));
        assert!(!scene.uses_advanced_layout(composer));
        for (viewport_height, input_height, composer_height) in [
            (None, 66., 114.),
            (Some(93.), 141., 189.),
            (Some(120.), 168., 216.),
            (None, 66., 114.),
            (Some(93.), 141., 189.),
        ] {
            let mut style = scene.style(attachments);
            style.height = Some(viewport_height.unwrap_or(93.));
            style.layout_options = viewport_height.is_none().then(|| {
                Arc::new(LayoutOptions {
                    display: Some(ViewDisplay::None),
                    ..Default::default()
                })
            });
            scene.set_style(attachments, style);
            let mut style = scene.style(tile);
            style.height = Some(viewport_height.unwrap_or(93.) - 33.);
            scene.set_style(tile, style);
            // The actual trace disagreed only on the unbounded probe. Exercise
            // both numeric available-height probes and restore the same cache key.
            for height in [None, Some(0.), Some(66.), None] {
                let constraints = Constraints {
                    width: Some(594.),
                    percent_width: Some(594.),
                    height,
                    ..Default::default()
                };
                assert_eq!(scene.measure(input, constraints), (594., input_height));
                assert_eq!(
                    scene.measure(composer, constraints),
                    (594., composer_height)
                );
            }
            scene.flush();
            for node in [wrapper, overlay, owner, composer] {
                assert_eq!(
                    scene.layout_bounds(node).height,
                    composer_height,
                    "{node:?}"
                );
                assert_eq!(scene.layout_bounds(node).y + composer_height, 400.);
            }
            let composer_bounds = scene.layout_bounds(composer);
            assert_eq!(scene.layout_bounds(body).height, input_height);
            assert_eq!(scene.layout_bounds(input).height, input_height);
            assert_eq!(scene.layout_bounds(stroke).height, input_height + 1.5);
            let footer_bounds = scene.layout_bounds(footer);
            assert_eq!(footer_bounds.y - composer_bounds.y, input_height + 9.);
            assert_eq!(footer_bounds.y + footer_bounds.height, 400.);
            assert_eq!(scene.flush().layout_nodes, 0);
        }
    }

    #[test]
    fn negative_margin_max_content_height_wraps_at_the_clamped_column_width() {
        use crate::text_layout::{FontStyle, LineHeight};
        let mut scene = Scene::new(400., 200.);
        let column = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(120.),
                max_width: Some(60.),
                layout_options: Some(Arc::new(crate::layout::LayoutOptions {
                    gap_y: Some(Length::Px(0.)),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let text = scene.append(
            column,
            NodeKind::Text {
                text: "abcdefghijabcdefghij".into(),
                font_size: 10.,
                color: Color(255, 255, 255, 255),
            },
            Style {
                text_wrap: true,
                margin: Insets {
                    top: -1.,
                    bottom: -1.,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        scene.set_font(
            text,
            FontStyle {
                line_height: LineHeight::px(10.),
                ..Default::default()
            },
        );
        for height in [None, Some(0.), None] {
            assert_eq!(
                scene.measure(
                    column,
                    Constraints {
                        height,
                        ..Default::default()
                    }
                ),
                (60., 18.)
            );
        }
        scene.flush();
        assert_eq!(scene.layout_bounds(column).height, 18.);
        assert_eq!(scene.layout_bounds(text).height, 20.);
    }

    #[test]
    fn negative_margin_intrinsic_height_keeps_percentage_children_indefinite() {
        use crate::layout::LayoutOptions;
        for case in 0..5 {
            let mut scene = Scene::new(400., 200.);
            let column = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Column),
                Style {
                    width: Some(200.),
                    layout_options: Some(Arc::new(LayoutOptions {
                        gap_x: Some(Length::Px(0.)),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            let mut options = LayoutOptions::default();
            match case {
                1 => options.min_height = Some(Length::Percent(1.)),
                2 => options.max_height = Some(Length::Percent(0.5)),
                3 => options.basis = Some(Length::Percent(1.)),
                4 => options.basis = Some(Length::Percent(0.)),
                _ => {}
            }
            let child = scene.append(
                column,
                NodeKind::Container(Layout::Column),
                Style {
                    height_percent: Some(1.),
                    margin: Insets {
                        bottom: -1.,
                        ..Default::default()
                    },
                    layout_options: Some(Arc::new(options)),
                    ..Default::default()
                },
            );
            let nested = scene.append(
                child,
                NodeKind::Container(Layout::Column),
                Style {
                    height_percent: Some(1.),
                    ..Default::default()
                },
            );
            scene.append(
                nested,
                NodeKind::Rect(Color(34, 51, 68, 255)),
                Style {
                    width: Some(20.),
                    height: Some(20.),
                    ..Default::default()
                },
            );
            let last = scene.append(
                column,
                NodeKind::Rect(Color(255, 255, 255, 255)),
                Style {
                    width: Some(20.),
                    height: Some(20.),
                    ..Default::default()
                },
            );
            for known in [false, true, false] {
                let constraints = Constraints {
                    force_height: known.then_some(100.),
                    ..Default::default()
                };
                assert_eq!(
                    scene.measure(column, constraints).1,
                    if known { 100. } else { 39. },
                    "case={case}, known={known}"
                );
                scene.arrange(column, 0., 0., &mut FrameReport::default(), false);
                assert_eq!(
                    scene.node(child).measurement.as_ref().unwrap().constraints.percent_height,
                    known.then_some(100.),
                    "case={case}, known={known}: preserve the original percentage basis"
                );
                let child_height = if known {
                    match case {
                        2 => 50.,
                        4 => 0.,
                        _ => 100.,
                    }
                } else {
                    20.
                };
                assert_eq!(
                    scene.layout_bounds(child).height,
                    child_height,
                    "case={case}, known={known}"
                );
                assert_eq!(scene.layout_bounds(nested).height, child_height);
                let last = scene.layout_bounds(last);
                assert_eq!(last.y, child_height - 1.);
                if !known {
                    assert_eq!(last.y + last.height, 39.);
                }
            }
        }
    }

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
    fn fixed_deep_layout_reuses_irrelevant_ancestor_constraints() {
        let (mut scene, containers, _, _, _) = deep_scene();
        let outer = containers[0];
        let base = Constraints { force_width: Some(160.), force_height: Some(700.), ..Default::default() };
        let expected = scene.measure(outer, base);
        scene.arrange(outer, 0., 0., &mut FrameReport::default(), false);
        let before = scene.measure_calls;
        for step in 1..=32 {
            let query = Constraints {
                width: Some(step as f32 * 17.), height: Some(step as f32 * 23.),
                percent_width: Some(step as f32 * 31.), percent_height: Some(step as f32 * 37.),
                ..base
            };
            assert_eq!(scene.measure(outer, query), expected);
            scene.arrange(outer, 0., 0., &mut FrameReport::default(), false);
        }
        assert_eq!(scene.measure_calls - before, 32, "equivalent probes must not descend the tree");
    }

    #[test]
    fn complete_layout_sizes_survive_geometry_cache_eviction() {
        let (mut scene, containers, _, _, _) = deep_scene();
        let outer = containers[0];
        let probes: Vec<_> = [96., 128., 160., 192., 256., 320.].into_iter().map(|width| Constraints {
            force_width: Some(width), ..Default::default()
        }).collect();
        let sizes: Vec<_> = probes.iter().map(|query| scene.measure(outer, *query)).collect();
        let before = scene.measure_calls;
        let arranged_from = scene.node(outer).measurement.as_ref().unwrap().constraints;
        for (query, expected) in probes.iter().zip(sizes) {
            assert_eq!(scene.measure_intrinsic(outer, *query), expected);
        }
        assert_eq!(scene.measure_calls, before, "size probes must reuse evicted complete layouts");
        assert_eq!(scene.node(outer).measurement.as_ref().unwrap().constraints, arranged_from);
    }

    #[test]
    fn normalized_constraints_preserve_percentage_bounds_padding_and_min_content() {
        fn build(style: Style) -> (Scene, NodeId) {
            let mut scene = Scene::new(400., 300.);
            scene.set_font_text_measurer(|text, size, width, font| {
                crate::text_layout::FallbackTextLayout::measure_with_font(text, size, width, font)
            });
            let parent = scene.append(scene.root(), NodeKind::Container(Layout::Column), style);
            scene.append(parent, label("a paragraph that wraps at the allocated width"), Style {
                text_wrap: true, ..Default::default()
            });
            scene.append(parent, NodeKind::Rect(Color(0, 0, 0, 255)), Style {
                width_percent: Some(0.5), height_percent: Some(0.5), ..Default::default()
            });
            (scene, parent)
        }
        for advanced in [false, true] {
            for case in 0..4 {
                let mut style = Style { width: Some(80.), height: Some(60.), ..Default::default() };
                let mut options = crate::layout::LayoutOptions::default();
                if case == 1 || case == 3 {
                    style.width = None;
                    style.height = None;
                    style.width_percent = Some(0.5);
                    style.height_percent = Some(0.5);
                }
                if case >= 2 {
                    options.min_width = Some(Length::Percent(0.3));
                    options.max_width = Some(Length::Percent(0.9));
                    options.min_height = Some(Length::Percent(0.2));
                    options.max_height = Some(Length::Percent(0.8));
                    options.padding = crate::layout::EdgeLengths::all(Length::Percent(0.05));
                }
                if advanced {
                    options.display = Some(ViewDisplay::Grid);
                    options.columns = Some(1);
                    options.wrap = Some(ViewWrap::NoWrap);
                }
                style.layout_options = (!options.is_empty()).then(|| Arc::new(options));
                let (mut warm, parent) = build(style.clone());
                for step in 0..81 {
                    let values = [None, Some(90.), Some(240.)];
                    let query = Constraints {
                        width: values[step % 3], height: values[(step / 3) % 3],
                        percent_width: values[(step / 9) % 3], percent_height: values[(step / 27) % 3],
                        force_width: (step % 7 == 0).then_some(80.),
                        force_height: (step % 11 == 0).then_some(60.),
                        min_content_width: step % 5 == 0,
                        ..Default::default()
                    };
                    let size = warm.measure(parent, query);
                    warm.arrange(parent, 0., 0., &mut FrameReport::default(), false);
                    let (mut cold, cold_parent) = build(style.clone());
                    assert_eq!(size, cold.measure(cold_parent, query), "advanced={advanced}, case={case}, step={step}");
                    cold.arrange(cold_parent, 0., 0., &mut FrameReport::default(), false);
                    assert_eq!(warm.padding(parent), cold.padding(cold_parent));
                    for (actual, expected) in warm.slots.iter().zip(&cold.slots) {
                        assert_eq!(actual.node.as_ref().unwrap().bounds, expected.node.as_ref().unwrap().bounds,
                            "advanced={advanced}, case={case}, step={step}");
                    }
                }
            }
        }
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
    fn intrinsic_column_reflow_preserves_width_height_and_percentage_contracts() {
        use crate::layout::LayoutOptions;
        use crate::text_layout::{FallbackTextLayout, FontStyle, LineHeight, WordWrap};

        let value = "aa aa aa aa aa aa aa aa aa aa aa aa";
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            line_height: LineHeight::px(20.),
            ..Default::default()
        };
        for advanced in [false, true] {
            for wrap in [false, true] {
                for (min_width, max_width) in [(None, None), (Some(160.), None), (None, Some(80.))]
                {
                    for fixed_height in [None, Some(200.)] {
                        let mut scene = Scene::new(400., 1000.);
                        scene.set_font_text_measurer(|text, size, width, font| {
                            FallbackTextLayout::measure_with_font(text, size, width, font)
                        });
                        let column = scene.append(
                            scene.root(),
                            NodeKind::Container(Layout::Column),
                            Style {
                                width: Some(196.),
                                height: fixed_height,
                                min_width,
                                max_width,
                                align: Align::Stretch,
                                layout_options: advanced.then(|| {
                                    Arc::new(LayoutOptions {
                                        gap_y: Some(Length::Px(0.)),
                                        ..Default::default()
                                    })
                                }),
                                ..Default::default()
                            },
                        );
                        scene.append(
                            column,
                            NodeKind::Rect(Color(0, 0, 0, 0)),
                            Style {
                                width: Some(120.),
                                height: Some(20.),
                                ..Default::default()
                            },
                        );
                        let text = scene.append(
                            column,
                            label(value),
                            Style {
                                text_wrap: wrap,
                                ..Default::default()
                            },
                        );
                        scene.set_font(text, font.clone());
                        let percent = scene.append(
                            column,
                            NodeKind::Container(Layout::Column),
                            Style {
                                width_percent: Some(0.5),
                                height_percent: Some(1.),
                                ..Default::default()
                            },
                        );
                        scene.append(
                            percent,
                            NodeKind::Rect(Color(0, 0, 0, 0)),
                            Style {
                                width: Some(10.),
                                height: Some(7.),
                                ..Default::default()
                            },
                        );
                        let text_minimum = FallbackTextLayout::measure_with_font(
                            value,
                            16.,
                            wrap.then_some(0.),
                            &font,
                        )
                        .0;
                        let minimum_width =
                            dimension(120_f32.max(text_minimum), min_width, max_width);
                        let minimum = Constraints {
                            width: Some(0.),
                            min_content_width: true,
                            ..Default::default()
                        };
                        for (query, expected_width) in [
                            (minimum, minimum_width),
                            (
                                Constraints {
                                    width: Some(400.),
                                    ..Default::default()
                                },
                                dimension(196., min_width, max_width),
                            ),
                            (
                                Constraints {
                                    width: Some(0.),
                                    force_width: Some(0.),
                                    ..Default::default()
                                },
                                dimension(0., min_width, max_width),
                            ),
                            (minimum, minimum_width),
                        ] {
                            let size = scene.measure(column, query);
                            assert!(
                                (size.0 - expected_width).abs() < 0.001,
                                "advanced={advanced} wrap={wrap} query={query:?} size={size:?}, expected width {expected_width}"
                            );
                            if fixed_height.is_none() {
                                let text_height = FallbackTextLayout::measure_with_font(
                                    value,
                                    16.,
                                    wrap.then_some(expected_width),
                                    &font,
                                )
                                .1;
                                assert!(
                                    (size.1 - 20. - text_height - 7.).abs() < 0.001,
                                    "advanced={advanced} wrap={wrap} query={query:?} size={size:?}, text height {text_height}"
                                );
                            } else {
                                assert_eq!(size.1, 200.);
                            }
                            scene.arrange(column, 0., 0., &mut FrameReport::default(), false);
                            if fixed_height.is_none() {
                                assert_eq!(scene.layout_bounds(percent).height, 7.);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wrapped_figures_use_caption_height_at_their_allocated_width() {
        use crate::layout::LayoutOptions;
        use crate::text_layout::{FallbackTextLayout, FontStyle, LineHeight, WordWrap};

        let mut scene = Scene::new(937., 5000.);
        scene.set_font_text_measurer(|text, size, width, font| {
            FallbackTextLayout::measure_with_font(text, size, width, font)
        });
        let board = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Row),
            Style {
                width: Some(937.),
                gap: 56.,
                align: Align::Start,
                layout_options: Some(Arc::new(LayoutOptions {
                    wrap: Some(ViewWrap::Wrap),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let descriptions = [
            "The Assistant lives at the top with its latest message and quick replies. Chats follow the desktop sidebar's structure, with search and new chat in thumb reach.",
            "Search and new chat stay in thumb reach.",
            "A longer caption wraps beneath the fixed-size device without changing the space between this row and the previous row of figures.",
        ];
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            line_height: LineHeight::px(21.),
            ..Default::default()
        };
        // Match the runtime's anonymous child regions, including the allocated
        // width bounds and percentage height that is indefinite during sizing.
        let region_style = |gap| Style {
            height_percent: Some(1.),
            min_width: Some(0.),
            align: Align::Stretch,
            layout_options: Some(Arc::new(LayoutOptions {
                display: Some(ViewDisplay::Flex),
                wrap: Some(ViewWrap::NoWrap),
                gap_x: Some(Length::Px(gap)),
                gap_y: Some(Length::Px(gap)),
                min_width: Some(Length::Percent(1.)),
                max_width: Some(Length::Percent(1.)),
                ..Default::default()
            })),
            ..Default::default()
        };
        let mut figures = Vec::new();
        for description in descriptions {
            let figure = scene.append(
                board,
                NodeKind::Container(Layout::Column),
                Style {
                    width: Some(417.),
                    flex_shrink: 1.,
                    align: Align::Stretch,
                    layout_options: Some(Arc::new(LayoutOptions {
                        min_width: Some(Length::Auto),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            let region = scene.append(
                figure,
                NodeKind::Container(Layout::Column),
                region_style(28.),
            );
            let device = scene.append(
                region,
                NodeKind::Container(Layout::Column),
                Style {
                    width: Some(417.),
                    height: Some(876.),
                    layout_options: Some(Arc::new(LayoutOptions {
                        min_height: Some(Length::Auto),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            let caption = scene.append(
                region,
                NodeKind::Container(Layout::Column),
                Style {
                    flex_shrink: 1.,
                    align: Align::Stretch,
                    padding_edges: Some(Insets {
                        left: 8.,
                        right: 8.,
                        ..Default::default()
                    }),
                    layout_options: Some(Arc::new(LayoutOptions {
                        min_height: Some(Length::Auto),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            let caption_region = scene.append(
                caption,
                NodeKind::Container(Layout::Column),
                region_style(8.),
            );
            scene.append(
                caption_region,
                NodeKind::Rect(Color(0, 0, 0, 0)),
                Style {
                    width: Some(190.),
                    height: Some(22.1),
                    ..Default::default()
                },
            );
            let text = scene.append(
                caption_region,
                NodeKind::Text {
                    text: description.into(),
                    color: Color(0, 0, 0, 255),
                    font_size: 14.,
                },
                Style {
                    text_wrap: true,
                    ..Default::default()
                },
            );
            scene.set_font(text, font.clone());
            let caption_height = 22.1
                + 8.
                + FallbackTextLayout::measure_with_font(description, 14., Some(401.), &font).1;
            figures.push((figure, device, caption, caption_height));
        }
        // MinContent requests are part of the enclosing wrapped flex sizing.
        // Repeating them also exercises restoration of alternate measurements.
        for _ in 0..2 {
            for &(figure, _, _, _) in &figures {
                let minimum = scene.measure_intrinsic(
                    figure,
                    Constraints {
                        width: Some(0.),
                        min_content_width: true,
                        ..Default::default()
                    },
                );
                assert!((minimum.0 - 417.).abs() < 0.001, "{minimum:?}");
            }
            let size = scene.measure(
                board,
                Constraints {
                    width: Some(937.),
                    force_width: Some(937.),
                    ..Default::default()
                },
            );
            scene.arrange(board, 0., 0., &mut FrameReport::default(), false);
            for &(figure, device, caption, caption_height) in &figures {
                let bounds = scene.layout_bounds(figure);
                let caption_bounds = scene.layout_bounds(caption);
                assert!((scene.layout_bounds(device).height - 876.).abs() < 0.001);
                assert!((caption_bounds.width - 417.).abs() < 0.001);
                assert!(
                    (caption_bounds.height - caption_height).abs() < 0.001,
                    "caption {caption_bounds:?}, expected {caption_height}"
                );
                assert!(
                    (bounds.height - 876. - 28. - caption_height).abs() < 0.001,
                    "figure {bounds:?}, caption {caption_bounds:?}"
                );
            }
            let row_height = 876. + 28. + figures[0].3.max(figures[1].3);
            assert!((scene.layout_bounds(figures[2].0).y - row_height - 56.).abs() < 0.001);
            let expected_height = row_height + 56. + 876. + 28. + figures[2].3;
            assert!(
                (size.1 - expected_height).abs() < 0.001,
                "board {size:?}, expected {expected_height}"
            );
        }
    }

    #[test]
    fn auto_grid_rows_keep_individual_card_content_heights_at_the_natural_allocation() {
        use crate::layout::{LayoutOptions, Length};
        let mut scene = Scene::new(937., 2000.);
        let grid = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(937.),
                gap: 12.,
                align: Align::Stretch,
                layout_options: Some(Arc::new(LayoutOptions {
                    display: Some(ViewDisplay::Grid),
                    columns: Some(2),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let mut cards = Vec::new();
        for (content_height, expected_height) in [(187.2, 343.2), (440., 596.), (274., 430.)] {
            let card = scene.append(
                grid,
                NodeKind::Container(Layout::Column),
                Style {
                    width_percent: Some(1.),
                    padding: 1.,
                    min_height: Some(300.),
                    align: Align::Stretch,
                    layout_options: Some(Arc::new(LayoutOptions {
                        column_full: Some(true),
                        align_self: Some(Align::Stretch),
                        gap_x: Some(Length::Px(0.)),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            let demo = scene.append(
                card,
                NodeKind::Container(Layout::Row),
                Style {
                    width_percent: Some(1.),
                    flex_grow: 1.,
                    flex_shrink: 1.,
                    align: Align::Center,
                    justify: Justify::Center,
                    padding_edges: Some(Insets {
                        top: 56.,
                        bottom: 56.,
                        left: 24.,
                        right: 24.,
                    }),
                    layout_options: Some(Arc::new(LayoutOptions {
                        basis: Some(Length::Percent(0.)),
                        gap_x: Some(Length::Px(16.)),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
            );
            let content = scene.append(
                demo,
                NodeKind::Rect(Color(34, 51, 68, 255)),
                Style {
                    width: Some(620.),
                    height: Some(content_height),
                    ..Default::default()
                },
            );
            let footer = scene.append(
                card,
                NodeKind::Rect(Color(255, 255, 255, 255)),
                Style {
                    width_percent: Some(1.),
                    height: Some(42.),
                    ..Default::default()
                },
            );
            cards.push((card, demo, content, footer, content_height, expected_height));
        }
        let total = cards.iter().map(|card| card.5).sum::<f32>() + 24.;
        // Giving the grid its already-natural height must not redistribute
        // height between unrelated auto rows or move each demo's centered content.
        for height in [None, Some(total), None, Some(total)] {
            let size = scene.measure(
                grid,
                Constraints {
                    width: Some(937.),
                    force_width: Some(937.),
                    height,
                    force_height: height,
                    ..Default::default()
                },
            );
            assert!((size.1 - total).abs() < 0.001, "grid {size:?}");
            scene.arrange(grid, 0., 0., &mut FrameReport::default(), false);
            let mut y = 0.;
            for &(card, demo, content, footer, content_height, expected_height) in &cards {
                let bounds = scene.layout_bounds(card);
                assert!(
                    (bounds.height - expected_height).abs() < 0.001,
                    "{height:?}: {bounds:?}, expected {expected_height}"
                );
                assert!((bounds.y - y).abs() < 0.001);
                let demo_bounds = scene.layout_bounds(demo);
                assert!((demo_bounds.height - content_height - 112.).abs() < 0.001);
                assert!((scene.layout_bounds(content).y - demo_bounds.y - 56.).abs() < 0.001);
                let footer_bounds = scene.layout_bounds(footer);
                assert!(
                    (footer_bounds.y + footer_bounds.height + 1. - bounds.y - bounds.height).abs()
                        < 0.001
                );
                y += expected_height + 12.;
            }
        }
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
