//! Retained scene graph. Mutations invalidate only their layout/paint/compositor dependencies.
use crate::affine::Affine;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
static NEXT_SCENE_ID: AtomicU64 = AtomicU64::new(1);

/// Add a rectangle to disjoint damage regions. Intersecting thin strips must
/// not inflate into their mostly-clean bounding box. GPU callers may also
/// merge nearby regions within `near_area` to reduce draw overhead.
pub fn merge_damage(regions: &mut Vec<Rect>, rect: Rect, near_area: Option<f32>) {
    let area = |r: Rect| r.width * r.height;
    // Subtract first, merge second. Interleaving them can repeatedly split
    // and regrow the same L shape without making progress.
    let mut pending = vec![rect];
    for existing in regions.iter() {
        pending = pending
            .into_iter()
            .flat_map(|rect| {
                let Some(overlap) = existing.intersection(rect) else {
                    return vec![rect];
                };
                [
                    Rect::new(rect.x, rect.y, rect.width, overlap.y - rect.y),
                    Rect::new(
                        rect.x,
                        overlap.y + overlap.height,
                        rect.width,
                        rect.y + rect.height - overlap.y - overlap.height,
                    ),
                    Rect::new(rect.x, overlap.y, overlap.x - rect.x, overlap.height),
                    Rect::new(
                        overlap.x + overlap.width,
                        overlap.y,
                        rect.x + rect.width - overlap.x - overlap.width,
                        overlap.height,
                    ),
                ]
                .into_iter()
                .filter(|band| band.width > 0. && band.height > 0.)
                .collect()
            })
            .collect();
    }
    regions.extend(pending);
    loop {
        let mut pair = None;
        'pairs: for a in 0..regions.len() {
            for b in a + 1..regions.len() {
                let union = regions[a].union(regions[b]);
                if area(union)
                    <= (area(regions[a]) + area(regions[b])) * 1.5 + near_area.unwrap_or(0.)
                    && !regions
                        .iter()
                        .enumerate()
                        .any(|(i, r)| i != a && i != b && r.intersects(union))
                {
                    pair = Some((a, b, union));
                    break 'pairs;
                }
            }
        }
        let Some((a, b, union)) = pair else {
            break;
        };
        regions[a] = union;
        regions.swap_remove(b);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    index: u32,
    generation: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let w = (self.x + self.width).min(other.x + other.width) - x;
        let h = (self.y + self.height).min(other.y + other.height) - y;
        (w > 0.0 && h > 0.0).then(|| Self::new(x, y, w, h))
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            (self.x + self.width).max(other.x + other.width) - x,
            (self.y + self.height).max(other.y + other.height) - y,
        )
    }
    pub fn expand(self, by: f32) -> Self {
        Self::new(
            self.x - by,
            self.y - by,
            self.width + 2.0 * by,
            self.height + 2.0 * by,
        )
    }
    fn translated(self, t: Transform) -> Self {
        Self::new(self.x + t.x, self.y + t.y, self.width, self.height)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effects {
    pub opacity: f32,
    pub blur_radius: f32,
    pub edge_fade: f32,
}
impl Default for Effects {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            blur_radius: 0.0,
            edge_fade: 0.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Row,
    Column,
    Overlay,
}
/// A rectangular outer shadow. Blur radius is the Gaussian sigma in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub color: Color,
    pub offset: Transform,
    pub blur_radius: f32,
    pub spread: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuadStyle {
    pub fill: Color,
    pub radius: f32,
    pub border_color: Color,
    pub border_width: f32,
    pub shadow: Option<BoxShadow>,
    pub decoration: Option<Arc<crate::decoration::Decoration>>,
}
impl Default for QuadStyle {
    fn default() -> Self {
        Self {
            fill: Color(0, 0, 0, 0),
            radius: 0.,
            border_color: Color(0, 0, 0, 0),
            border_width: 0.,
            shadow: None,
            decoration: None,
        }
    }
}
impl QuadStyle {
    pub fn shadows(&self) -> &[BoxShadow] {
        self.decoration
            .as_ref()
            .and_then(|d| d.shadows.as_deref())
            .unwrap_or(self.shadow.as_slice())
    }
    pub fn paint_bounds(&self, bounds: Rect) -> Rect {
        self.shadows().iter().fold(bounds, |paint, shadow| {
            paint.union(
                bounds
                    .expand(shadow.spread.max(0.) + shadow.blur_radius.max(0.) * 3.)
                    .translated(shadow.offset),
            )
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Container(Layout),
    /// A decorated container; its background does not participate in child layout.
    Panel {
        layout: Layout,
        quad: QuadStyle,
    },
    Rect(Color),
    Quad(QuadStyle),
    Image(Arc<crate::image::ImageData>),
    Svg(Arc<crate::svg::SvgData>),
    Canvas(Arc<crate::canvas::Canvas>),
    #[cfg(target_os = "macos")]
    NativeSurface(std::rc::Rc<crate::native_surface::NativeSurface>),
    RichText {
        text: Arc<crate::rich_text::RichText>,
    },
    Text {
        text: Arc<str>,
        color: Color,
        font_size: f32,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}
impl Insets {
    pub fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    Baseline,
    #[default]
    Start,
    Center,
    End,
    Stretch,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
/// A platform text shaper can provide accurate intrinsic and wrapped text metrics.
pub trait TextMeasurer {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32);
}
impl<F: Fn(&str, f32, Option<f32>) -> (f32, f32)> TextMeasurer for F {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self(text, font_size, max_width)
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    /// Opt-in grid, wrapping, richer lengths and positioning.
    pub layout_options: Option<Arc<crate::layout::LayoutOptions>>,
    /// Remove this child from normal flow and position it at its parent content
    /// origin plus margins. It does not contribute to intrinsic parent size.
    pub absolute: bool,
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Fraction of the definite parent content width (1.0 means 100%).
    /// Falls back to intrinsic sizing when that axis is indefinite. Pixel width wins.
    pub width_percent: Option<f32>,
    /// Fraction of the definite parent content height. Absolute children use the
    /// final parent box. Invalid/negative fractions resolve to zero; values over 1 are allowed.
    pub height_percent: Option<f32>,
    pub padding: f32,
    /// Per-edge padding, overriding `padding` when present.
    pub padding_edges: Option<Insets>,
    pub gap: f32,
    pub clip: bool,
    /// Fade descendants out over these bands (top, bottom) at the edges of
    /// this node's clip, in the fragment shader: a scroll view whose content
    /// dissolves under a titlebar or into a composer, without a layer.
    pub fade_edges: [f32; 2],
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub margin: Insets,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub align: Align,
    pub justify: Justify,
    /// Request wrapping at the available content width from the text measurer.
    pub text_wrap: bool,
    /// Plain-text display overflow; ignored by editor document shaping.
    /// `NodeKind::RichText` instead uses its immutable `RichText::options()` so
    /// measurement, rendering and actionable span geometry share one source.
    pub text_options: crate::text_layout::TextOptions,
}

const LAYOUT: u8 = 1;
const PAINT: u8 = 2;
const COMPOSITE: u8 = 4;
struct Node {
    cursor: Option<crate::cursor::Cursor>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    kind: NodeKind,
    font: crate::text_layout::FontStyle,
    style: Style,
    bounds: Rect,
    transform: Transform,
    paint_transform: Affine,
    paint_origin: [f32; 2],
    projection_transform: Affine,
    effects: Effects,
    dirty: u8,
    queued: bool,
    /// The measurement `arrange` places children from: the most recent one used.
    measurement: Option<Measurement>,
    /// Other recent results under different constraints, most recent first.
    alternates: Vec<Measurement>,
    /// Everything this subtree can paint, including its own translation but not
    /// its ancestors'. Rendering skips subtrees whose ink misses the damage.
    /// A valid visible node has only valid descendants, so invalidation stops
    /// at the first invalid ancestor. A hidden node is cached as painting
    /// nothing without visiting its children; it can only become visible
    /// through its own change, which invalidates it and its ancestors.
    ink: std::cell::Cell<Option<Rect>>,
    ink_valid: std::cell::Cell<bool>,
    /// Intrinsic text or rich text size by wrap limit and shaper revision. Flex layout
    /// measures a leaf under several constraint sets per pass, which defeats
    /// `measurement`; this keeps an unchanged label from being reshaped each
    /// time an ancestor relayouts. Cleared by the node's own layout invalidation.
    text_size: TextSizes,
    advanced_layout: Option<Box<advanced_layout::AdvancedLayoutCache>>,
    arrange_dirty: bool,
    isolated: bool,
    layer_revision: u64,
    paint_revision: u64,
    /// Scroll content: a translation is reported as a `ScrollMove` the
    /// renderer may apply by copying pixels (see `set_scroll_copy`).
    scroll_copy: bool,
}
/// A text node's measured sizes for its most recent wrap limits. Layout
/// measures a paragraph at more than one width per pass (intrinsic, then
/// final); one entry would evict the other and reshape the text each time.
#[derive(Clone, Copy, Default)]
struct TextSizes([Option<TextSize>; 3]);
/// A wrap limit, the shaper revision it was measured under, and the size.
type TextSize = (Option<f32>, u64, (f32, f32));
impl TextSizes {
    fn get(&self, limit: Option<f32>, revision: u64) -> Option<(f32, f32)> {
        self.0
            .iter()
            .flatten()
            .find(|(cached, cached_revision, _)| *cached == limit && *cached_revision == revision)
            .map(|(_, _, size)| *size)
    }
    fn insert(&mut self, limit: Option<f32>, revision: u64, size: (f32, f32)) {
        self.0.rotate_right(1);
        self.0[0] = Some((limit, revision, size));
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Constraints {
    width: Option<f32>,
    height: Option<f32>,
    percent_width: Option<f32>,
    percent_height: Option<f32>,
    force_width: Option<f32>,
    force_height: Option<f32>,
    // An ancestor with display:none suppresses this whole subtree's layout.
    hidden: bool,
}
struct Measurement {
    constraints: Constraints,
    size: (f32, f32),
    children: Vec<(NodeId, Rect)>,
    /// The constraints each child was last measured with for this result. A
    /// later probe (a flex basis, an intrinsic size) can leave a child holding
    /// another result; `arrange` restores this one before placing it.
    child_constraints: Vec<Option<Constraints>>,
    /// Layout pass that produced this result. A dirty node may reuse only
    /// results from the pass in progress.
    pass: u64,
}
/// Flex layout measures a child with its natural, flexed and stretched
/// constraints in one pass. Keeping a few results lets a clean child answer all
/// of them instead of re-measuring its whole subtree each time an ancestor lays out.
const MEASUREMENT_ALTERNATES: usize = 3;
fn dimension(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    value
        .max(0.0)
        .min(max.unwrap_or(f32::INFINITY).max(0.0))
        .max(min.unwrap_or(0.0).max(0.0))
}
fn percentage(fraction: Option<f32>, basis: Option<f32>) -> Option<f32> {
    fraction.zip(basis).map(|(fraction, basis)| {
        let fraction = if fraction.is_finite() {
            fraction.max(0.0)
        } else {
            0.0
        };
        ((fraction as f64 * basis.max(0.0) as f64).min(f32::MAX as f64)) as f32
    })
}
struct Slot {
    generation: u32,
    node: Option<Node>,
}
#[derive(Default, Debug)]
pub struct FrameReport {
    pub damage: Vec<Rect>,
    pub layout_nodes: usize,
    pub paint_nodes: usize,
    pub composite_nodes: usize,
}
impl FrameReport {
    pub fn is_idle(&self) -> bool {
        self.damage.is_empty()
            && self.layout_nodes == 0
            && self.paint_nodes == 0
            && self.composite_nodes == 0
    }
}
/// The parts of `clip` that content shifted by `(dx, dy)` leaves uncovered.
fn exposed(clip: Rect, dx: f32, dy: f32) -> Vec<Rect> {
    let mut strips = Vec::new();
    if dy > 0. {
        strips.push(Rect::new(clip.x, clip.y, clip.width, dy));
    } else if dy < 0. {
        strips.push(Rect::new(
            clip.x,
            clip.y + clip.height + dy,
            clip.width,
            -dy,
        ));
    }
    if dx > 0. {
        strips.push(Rect::new(clip.x, clip.y, dx, clip.height));
    } else if dx < 0. {
        strips.push(Rect::new(
            clip.x + clip.width + dx,
            clip.y,
            -dx,
            clip.height,
        ));
    }
    strips
}
/// A `fade_edges` ancestor's clip and its (top, bottom) bands.
pub type FadeMask = (Rect, [f32; 2]);
/// Exact clipping in an ancestor's pre-transform layout coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintClip {
    pub bounds: Rect,
    pub inverse: Affine,
    pub axes: [bool; 2],
    pub corners: [f32; 4],
}
impl PaintClip {
    fn distance(self, x: f32, y: f32) -> f32 {
        let (x, y) = self.inverse.point(x, y);
        let dx = x - self.bounds.x - self.bounds.width * 0.5;
        let dy = y - self.bounds.y - self.bounds.height * 0.5;
        let index = match (dx > 0., dy > 0.) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        };
        let r = self.corners[index];
        let qx = dx.abs() - self.bounds.width * 0.5 + r;
        let qy = dy.abs() - self.bounds.height * 0.5 + r;
        qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - r
    }
    pub fn coverage(self, x: f32, y: f32) -> f32 {
        let (qx, qy) = self.inverse.point(x, y);
        if (!self.axes[0] || (qx >= self.bounds.x && qx < self.bounds.x + self.bounds.width))
            && (!self.axes[1] || (qy >= self.bounds.y && qy < self.bounds.y + self.bounds.height))
        {
            if self.corners.iter().all(|r| *r == 0.) {
                return 1.;
            }
            let pixel = (self.inverse.a.abs() + self.inverse.c.abs())
                .max(self.inverse.b.abs() + self.inverse.d.abs())
                .max(0.5);
            (0.5 - self.distance(x, y) / pixel).clamp(0., 1.)
        } else {
            0.
        }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        let (qx, qy) = self.inverse.point(x, y);
        (!self.axes[0] || (qx >= self.bounds.x && qx < self.bounds.x + self.bounds.width))
            && (!self.axes[1] || (qy >= self.bounds.y && qy < self.bounds.y + self.bounds.height))
            && (self.corners.iter().all(|r| *r == 0.) || self.distance(x, y) <= 0.)
    }
}
#[derive(Clone)]
pub struct PaintItem<'a> {
    /// An explicit isolated subtree represented by its cached layer image.
    pub isolated: bool,
    pub id: NodeId,
    pub bounds: Rect,
    /// Maps the primitive allocation to the renderer's target coordinates.
    /// Pure translations are baked into `bounds` and retain the identity fast path.
    pub transform: Affine,
    pub clip_regions: std::rc::Rc<[PaintClip]>,
    /// Maps target coordinates back into `mask` coordinates.
    pub fade_transform: Affine,
    pub kind: &'a NodeKind,
    pub font: &'a crate::text_layout::FontStyle,
    pub text_options: crate::text_layout::TextOptions,
    pub effects: Effects,
    pub clip: Option<Rect>,
    /// The nearest `fade_edges` ancestor's clip and bands (top, bottom).
    pub mask: Option<FadeMask>,
}
type FontMeasurer =
    dyn Fn(&std::sync::Arc<str>, f32, Option<f32>, &crate::text_layout::FontStyle) -> (f32, f32);
pub struct Scene {
    identity: u64,
    tree_revision: u64,
    visibility_revision: u64,
    raster_revision: u64,
    scroll_copy_count: usize,
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: NodeId,
    dirty: Vec<NodeId>,
    damage: Vec<Rect>,
    viewport: Rect,
    blur_nodes: Vec<NodeId>,
    isolation_count: usize,
    text_measurer: Option<Box<dyn TextMeasurer>>,
    text_shaper: Option<Box<dyn crate::text_layout::TextShaper>>,
    font_shaper: Option<Box<dyn crate::text_layout::FontTextShaper>>,
    font_measurer: Option<Box<FontMeasurer>>,
    rich_measurer: crate::rich_text::TextMeasure,
    rich_shaper: Option<Box<crate::rich_text::RichShaper>>,
    text_shaper_revision: u64,
    pending_layout_nodes: usize,
    layout_revision: u64,
    layout_pass: u64,
    content_revision: u64,
    /// Dirty fixed-size containers whose ancestors stayed clean.
    layout_roots: Vec<NodeId>,
    geometry_revision: u64,
    cursor_styles: usize,
    /// Scroll content translated since the last flush: its transform before
    /// the first move, and whether it moved again (see `resolve_moves`).
    pending_moves: Vec<(NodeId, Transform, bool)>,
    /// The moves reported by the last flush, and that flush's serial.
    moves: Vec<ScrollMove>,
    flush_serial: u64,
}
/// Scroll content that moved rigidly by `(dx, dy)` within `clip`. The flush
/// damaged only what copying the old pixels cannot provide (`exposed`, plus
/// every other change inside the clip at both positions). A renderer that
/// cannot copy must repaint `clip` as well.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollMove {
    pub node: NodeId,
    pub clip: Rect,
    pub dx: f32,
    pub dy: f32,
    /// `flush_serial` of the flush that reported it: the copy is valid only
    /// over the frame rendered from the previous flush.
    pub serial: u64,
}
impl Scene {
    pub fn new(width: f32, height: f32) -> Self {
        let root = NodeId {
            index: 0,
            generation: 0,
        };
        Self {
            identity: NEXT_SCENE_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("scene identity exhausted"),
            slots: vec![Slot {
                generation: 0,
                node: Some(Node {
                    cursor: None,
                    parent: None,
                    children: Vec::new(),
                    kind: NodeKind::Container(Layout::Column),
                    font: Default::default(),
                    style: Style {
                        width: Some(width),
                        height: Some(height),
                        clip: true,
                        ..Style::default()
                    },
                    bounds: Rect::default(),
                    transform: Transform::default(),
                    paint_transform: Affine::IDENTITY,
                    paint_origin: [0., 0.],
                    projection_transform: Affine::IDENTITY,
                    effects: Effects::default(),
                    dirty: LAYOUT | PAINT,
                    queued: true,
                    measurement: None,
                    alternates: Vec::new(),
                    ink: std::cell::Cell::new(None),
                    ink_valid: std::cell::Cell::new(false),
                    text_size: TextSizes::default(),
                    advanced_layout: None,
                    arrange_dirty: true,
                    isolated: false,
                    scroll_copy: false,
                    layer_revision: 0,
                    paint_revision: 0,
                }),
            }],
            free: Vec::new(),
            root,
            dirty: vec![root],
            damage: Vec::new(),
            viewport: Rect::new(0.0, 0.0, width, height),
            blur_nodes: Vec::new(),
            pending_moves: Vec::new(),
            moves: Vec::new(),
            flush_serial: 0,
            visibility_revision: 0,
            tree_revision: 0,
            raster_revision: 0,
            scroll_copy_count: 0,
            isolation_count: 0,
            text_measurer: None,
            text_shaper: None,
            font_shaper: None,
            font_measurer: None,
            rich_measurer: Default::default(),
            rich_shaper: None,
            text_shaper_revision: 0,
            pending_layout_nodes: 0,
            layout_revision: 0,
            layout_pass: 0,
            content_revision: 0,
            layout_roots: Vec::new(),
            geometry_revision: 0,
            cursor_styles: 0,
        }
    }
    /// Stable identity separates renderer caches when a whole scene is replaced.
    pub fn identity(&self) -> u64 {
        self.identity
    }
    pub fn root(&self) -> NodeId {
        self.root
    }
    pub fn contains(&self, id: NodeId) -> bool {
        self.slots
            .get(id.index as usize)
            .is_some_and(|s| s.generation == id.generation && s.node.is_some())
    }
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }
    pub fn is_empty(&self) -> bool {
        false
    } // A scene always retains its root.
    fn node(&self, id: NodeId) -> &Node {
        assert!(self.contains(id), "stale scene node");
        self.slots[id.index as usize].node.as_ref().unwrap()
    }
    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        assert!(self.contains(id), "stale scene node");
        self.slots[id.index as usize].node.as_mut().unwrap()
    }
    pub fn bounds(&self, id: NodeId) -> Rect {
        self.world_paint_transform(id).bounds(self.node(id).bounds)
    }
    /// Settled layout allocation, excluding every paint translation.
    pub fn layout_bounds(&self, id: NodeId) -> Rect {
        self.node(id).bounds
    }
    /// Visible allocation after ancestor opacity, clipping and viewport intersection.
    /// This does not attempt occlusion testing against overlapping siblings.
    /// Layout visibility excludes hidden/display-none nodes and their descendants.
    /// Mounted ownership is retained; opacity remains an independent paint effect.
    pub fn layout_visible(&self, id: NodeId) -> bool {
        self.contains(id)
            && std::iter::once(id)
                .chain(self.ancestors(id))
                .all(|id| own_visible(&self.node(id).style))
    }
    pub fn visible_bounds(&self, id: NodeId) -> Option<Rect> {
        let (bounds, effects, clip) = self.world(id);
        if effects.opacity <= 0. {
            return None;
        }
        bounds.intersection(clip.unwrap_or(self.viewport))
    }
    /// An explicit cursor inherits through visual ancestors, including disabled regions.
    pub fn cursor_for(&self, mut id: NodeId) -> Option<crate::cursor::Cursor> {
        loop {
            let node = self.node(id);
            if node.cursor.is_some() {
                return node.cursor;
            }
            id = node.parent?;
        }
    }
    pub fn set_cursor(&mut self, id: NodeId, cursor: Option<crate::cursor::Cursor>) {
        let old = self.node(id).cursor;
        if old != cursor {
            self.cursor_styles =
                self.cursor_styles - usize::from(old.is_some()) + usize::from(cursor.is_some());
            self.node_mut(id).cursor = cursor;
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
    }
    pub fn interaction_revision(&self) -> u64 {
        self.geometry_revision
    }
    pub fn cursor_at(&self, x: f32, y: f32) -> Option<crate::cursor::Cursor> {
        if self.cursor_styles == 0 {
            return None;
        }
        self.hit_test_all(x, y)
            .first()
            .and_then(|id| self.cursor_for(*id))
    }
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }
    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.node(id).kind
    }
    pub fn style(&self, id: NodeId) -> Style {
        self.node(id).style.clone()
    }
    pub fn effects(&self, id: NodeId) -> Effects {
        self.node(id).effects
    }
    pub fn transform(&self, id: NodeId) -> Transform {
        self.node(id).transform
    }
    pub fn paint_transform(&self, id: NodeId) -> Affine {
        self.node(id).paint_transform
    }
    pub fn paint_transform_origin(&self, id: NodeId) -> [f32; 2] {
        self.node(id).paint_origin
    }
    pub fn projection_transform(&self, id: NodeId) -> Affine {
        self.node(id).projection_transform
    }
    /// Style affine and translation in node-local coordinates, around the
    /// projected normalized pivot; excludes the projection matrix itself.
    pub fn local_user_transform(&self, id: NodeId) -> Affine {
        let node = self.node(id);
        let (x, y) = node.projection_transform.point(
            node.bounds.width * node.paint_origin[0],
            node.bounds.height * node.paint_origin[1],
        );
        node.paint_transform
            .around(x, y)
            .then(Affine::translation(node.transform.x, node.transform.y))
    }
    pub fn local_paint_transform(&self, id: NodeId) -> Affine {
        self.node(id)
            .projection_transform
            .then(self.local_user_transform(id))
    }
    /// Resolve a projection from the desired complete node-local paint matrix,
    /// retaining the node's style affine and normalized pivot. A singular style
    /// matrix has no unique solution and uses identity projection.
    pub fn projection_for_paint(&self, id: NodeId, desired: Affine) -> Affine {
        let node = self.node(id);
        let (x, y) = desired.point(
            node.bounds.width * node.paint_origin[0],
            node.bounds.height * node.paint_origin[1],
        );
        let user = node.paint_transform.around(
            x - node.transform.x - node.paint_transform.tx,
            y - node.transform.y - node.paint_transform.ty,
        );
        let Some(inverse) = user.inverse() else {
            return Affine::IDENTITY;
        };
        let projection = desired
            .then(Affine::translation(-node.transform.x, -node.transform.y))
            .then(inverse);
        if projection.is_finite() {
            projection
        } else {
            Affine::IDENTITY
        }
    }
    fn node_matrix(&self, id: NodeId) -> Affine {
        let node = self.node(id);
        if node.paint_transform.is_translation() && node.projection_transform.is_translation() {
            return Affine::translation(
                node.paint_transform.tx + node.projection_transform.tx + node.transform.x,
                node.paint_transform.ty + node.projection_transform.ty + node.transform.y,
            );
        }
        self.local_paint_transform(id)
            .around(node.bounds.x, node.bounds.y)
    }
    /// Maps absolute, unpainted layout coordinates into world coordinates.
    pub fn world_paint_transform(&self, id: NodeId) -> Affine {
        let mut matrix = self.node_matrix(id);
        for ancestor in self.ancestors(id) {
            matrix = matrix.then(self.node_matrix(ancestor));
        }
        matrix
    }
    pub fn paint_matrix(&self, id: NodeId) -> Affine {
        self.world_paint_transform(id)
    }
    /// User affine hierarchy without projection linear terms, retaining the
    /// actual projected pivots. Used to factor nested layout projection without
    /// introducing jumps at rotated or scaled intermediate containers.
    pub fn world_user_transform(&self, id: NodeId) -> Affine {
        let mut path: Vec<_> = std::iter::once(id).chain(self.ancestors(id)).collect();
        path.reverse();
        let mut full = Affine::IDENTITY;
        let mut user = Affine::IDENTITY;
        for current in path {
            let node = self.node(current);
            full = self.node_matrix(current).then(full);
            let (x, y) = full.point(
                node.bounds.x + node.bounds.width * node.paint_origin[0],
                node.bounds.y + node.bounds.height * node.paint_origin[1],
            );
            let Some(inverse) = user.inverse() else {
                return Affine::IDENTITY;
            };
            let (x, y) = inverse.point(x, y);
            let (tx, ty) = (
                node.paint_transform.tx + node.transform.x,
                node.paint_transform.ty + node.transform.y,
            );
            user = Affine {
                tx: 0.,
                ty: 0.,
                ..node.paint_transform
            }
            .around(x - tx, y - ty)
            .then(Affine::translation(tx, ty))
            .then(user);
        }
        user
    }
    /// Projection only, excluding style translation, scale and rotation.
    pub fn world_projection_transform(&self, id: NodeId) -> Affine {
        let mut matrix = Affine::IDENTITY;
        for current in std::iter::once(id).chain(self.ancestors(id)) {
            let node = self.node(current);
            matrix = matrix.then(
                node.projection_transform
                    .around(node.bounds.x, node.bounds.y),
            );
        }
        matrix
    }
    pub fn local_to_world(&self, id: NodeId, x: f32, y: f32) -> (f32, f32) {
        let bounds = self.node(id).bounds;
        self.world_paint_transform(id)
            .point(x + bounds.x, y + bounds.y)
    }
    pub fn world_to_local(&self, id: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        let (x, y) = self.world_paint_transform(id).inverse()?.point(x, y);
        let bounds = self.node(id).bounds;
        Some((x - bounds.x, y - bounds.y))
    }
    /// Ancestors from the immediate parent to the root.
    pub fn ancestors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(id), |id| self.parent(*id))
    }
    /// Front-to-back hit targets, including containers; call after flushing layout.
    pub fn hit_test_all(&self, x: f32, y: f32) -> Vec<NodeId> {
        let mut hits = Vec::new();
        for item in self.paint_items() {
            if item.effects.opacity <= 0.
                || item.clip.is_some_and(|clip| !contains_point(clip, x, y))
                || item.clip_regions.iter().any(|clip| !clip.contains(x, y))
            {
                continue;
            }
            let primitive = match item.kind {
                NodeKind::Image(image) => image.paint_transform(item.bounds),
                NodeKind::Svg(svg) => svg.paint_transform(item.bounds),
                _ => Affine::IDENTITY,
            }
            .then(item.transform);
            if primitive.inverse().is_some_and(|inverse| {
                let (x, y) = inverse.point(x, y);
                contains_point(item.bounds, x, y)
            }) {
                hits.push(item.id);
            }
        }
        hits.reverse();
        hits
    }
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit_test_all(x, y).into_iter().next()
    }
    /// Uses the same installed font measurer as layout, useful for caret geometry.
    pub fn measure_text(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        if let Some(shaper) = &self.font_shaper {
            return shaper
                .shape(text, font_size, max_width, &Default::default())
                .size();
        }
        if let Some(shaper) = &self.text_shaper {
            return shaper.shape(text, font_size, max_width).size();
        }
        if let Some(measurer) = &self.text_measurer {
            return measurer.measure(text, font_size, max_width);
        }
        crate::text_layout::FallbackTextLayout::measure(
            text,
            font_size,
            max_width,
            crate::text_layout::LineHeight::NORMAL,
        )
    }
    /// Reorders an exact permutation of a parent's children, retaining every ID.
    /// Invalid parents, foreign nodes, omissions, and duplicates return `false`
    /// without modifying the scene. An unchanged order is a successful no-op.
    pub fn reorder_children(&mut self, parent: NodeId, children: &[NodeId]) -> bool {
        if !self.contains(parent) {
            return false;
        }
        let current = &self.node(parent).children;
        if current == children {
            return true;
        }
        if current.len() != children.len() {
            return false;
        }
        let unique: std::collections::HashSet<_> = children.iter().copied().collect();
        if unique.len() != children.len() || !current.iter().all(|id| unique.contains(id)) {
            return false;
        }
        self.damage_subtree(parent);
        self.node_mut(parent).children.copy_from_slice(children);
        self.tree_revision = self.tree_revision.wrapping_add(1);
        self.invalidate_layout(parent);
        true
    }
    /// Installs the native font engine for editor geometry and intrinsic measurement.
    pub fn set_text_shaper(&mut self, shaper: impl crate::text_layout::TextShaper + 'static) {
        self.text_shaper_revision = self.text_shaper_revision.wrapping_add(1);
        self.text_shaper = Some(Box::new(shaper));
        let ids: Vec<_> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                slot.node.as_ref().map(|_| NodeId {
                    index: index as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for id in ids {
            self.invalidate(id, LAYOUT | PAINT);
        }
    }
    pub fn shape_text(
        &self,
        text: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> Box<dyn crate::text_layout::TextLayout> {
        if let Some(shaper) = &self.font_shaper {
            shaper.shape(text, font_size, max_width, &Default::default())
        } else if let Some(shaper) = &self.text_shaper {
            shaper.shape(text, font_size, max_width)
        } else {
            Box::new(crate::text_layout::FallbackTextLayout::new(
                text, font_size, max_width,
            ))
        }
    }
    pub fn set_text_measurer(&mut self, measurer: impl TextMeasurer + 'static) {
        self.text_measurer = Some(Box::new(measurer));
        self.text_shaper_revision = self.text_shaper_revision.wrapping_add(1);
        let ids: Vec<_> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                slot.node.as_ref().map(|_| NodeId {
                    index: index as u32,
                    generation: slot.generation,
                })
            })
            .collect();
        for id in ids {
            self.invalidate(id, LAYOUT | PAINT);
        }
    }
    pub fn font(&self, id: NodeId) -> &crate::text_layout::FontStyle {
        &self.node(id).font
    }
    /// Font changes affect shaping, layout, paint and retained layer contents.
    pub fn set_font(&mut self, id: NodeId, mut font: crate::text_layout::FontStyle) {
        font.weight = font.weight.clamp(1, 1000);
        if self.node(id).font == font {
            return;
        }
        self.damage_subtree(id);
        self.node_mut(id).font = font;
        self.invalidate(id, PAINT);
        self.invalidate_layout(id);
    }
    pub fn set_font_text_shaper(
        &mut self,
        shaper: impl crate::text_layout::FontTextShaper + 'static,
    ) {
        self.text_shaper_revision = self.text_shaper_revision.wrapping_add(1);
        self.font_shaper = Some(Box::new(shaper));
        let ids: Vec<_> = self
            .paint_items()
            .filter(|item| matches!(item.kind, NodeKind::Text { .. }))
            .map(|item| item.id)
            .collect();
        for id in ids {
            self.invalidate(id, PAINT);
            self.invalidate_layout(id);
        }
    }
    /// Install rich paragraph geometry for inline selection and interactive spans.
    pub fn set_rich_text_shaper(
        &mut self,
        shaper: impl Fn(
            &crate::rich_text::RichText,
            Option<f32>,
        ) -> Box<dyn crate::text_layout::TextLayout>
        + 'static,
    ) {
        self.rich_shaper = Some(Box::new(shaper));
        self.text_shaper_revision = self.text_shaper_revision.wrapping_add(1);
    }
    pub fn shape_rich_text(
        &self,
        rich: &crate::rich_text::RichText,
        width: Option<f32>,
    ) -> Box<dyn crate::text_layout::TextLayout> {
        self.rich_shaper.as_ref().map_or_else(
            || {
                Box::new(crate::text_layout::FallbackTextLayout::with_rich(
                    rich, width,
                )) as Box<dyn crate::text_layout::TextLayout>
            },
            |shaper| shaper(rich, width),
        )
    }
    /// Install measurement for plain text nodes, sized as the installed font
    /// shaper lays them out. Hosts measure from the text they draw from.
    pub fn set_font_text_measurer(
        &mut self,
        measure: impl Fn(
            &std::sync::Arc<str>,
            f32,
            Option<f32>,
            &crate::text_layout::FontStyle,
        ) -> (f32, f32)
        + 'static,
    ) {
        self.font_measurer = Some(Box::new(measure));
        self.text_shaper_revision = self.text_shaper_revision.wrapping_add(1);
        let ids: Vec<_> = self
            .paint_items()
            .filter(|item| matches!(item.kind, NodeKind::Text { .. }))
            .map(|item| item.id)
            .collect();
        for id in ids {
            self.invalidate(id, PAINT);
            self.invalidate_layout(id);
        }
    }
    /// Install native continuous mixed-run measurement for rich display text.
    pub fn set_rich_text_measurer(
        &mut self,
        measure: impl Fn(&crate::rich_text::RichText, Option<f32>) -> (f32, f32) + 'static,
    ) {
        self.rich_measurer.install(Box::new(measure));
        self.text_shaper_revision = self.text_shaper_revision.wrapping_add(1);
        let ids: Vec<_> = self
            .paint_items()
            .filter(|item| matches!(item.kind, NodeKind::RichText { .. }))
            .map(|item| item.id)
            .collect();
        for id in ids {
            self.invalidate(id, PAINT);
            self.invalidate_layout(id);
        }
    }
    /// Install measurement for rich text that is not mounted (`TextMeasure`),
    /// sized exactly as `set_rich_text_measurer` sizes it but without keeping
    /// what drawing it would need.
    pub fn set_detached_rich_text_measurer(
        &mut self,
        measure: impl Fn(&crate::rich_text::RichText, Option<f32>) -> (f32, f32) + 'static,
    ) {
        self.rich_measurer.install_detached(Box::new(measure));
    }
    /// The rich text measurer layout uses, including one installed later.
    pub fn text_measure(&self) -> crate::rich_text::TextMeasure {
        self.rich_measurer.clone()
    }
    /// Changes whenever either installed shaping callback is replaced.
    pub(crate) fn text_shaper_revision(&self) -> u64 {
        self.text_shaper_revision
    }
    pub fn shape_text_with_font(
        &self,
        text: &str,
        size: f32,
        width: Option<f32>,
        font: &crate::text_layout::FontStyle,
    ) -> Box<dyn crate::text_layout::TextLayout> {
        if let Some(shaper) = &self.font_shaper {
            shaper.shape(text, size, width, font)
        } else if font.line_height != crate::text_layout::LineHeight::NORMAL
            || font.letter_spacing != crate::text_layout::LetterSpacing::ZERO
        {
            // Legacy callbacks cannot receive font metrics. Use the same
            // grapheme-cell fallback for measuring and editing explicit metrics.
            Box::new(crate::text_layout::FallbackTextLayout::with_font(
                text, size, width, font,
            ))
        } else {
            self.shape_text(text, size, width)
        }
    }
    pub fn measure_text_with_font(
        &self,
        text: &str,
        size: f32,
        width: Option<f32>,
        font: &crate::text_layout::FontStyle,
    ) -> (f32, f32) {
        if let Some(shaper) = &self.font_shaper {
            shaper.shape(text, size, width, font).size()
        } else if font.line_height != crate::text_layout::LineHeight::NORMAL
            || font.letter_spacing != crate::text_layout::LetterSpacing::ZERO
        {
            crate::text_layout::FallbackTextLayout::measure_with_font(text, size, width, font)
        } else {
            self.measure_text(text, size, width)
        }
    }
    pub fn set_kind(&mut self, id: NodeId, kind: NodeKind) {
        if self.node(id).kind == kind {
            return;
        }
        // Renderer caches are keyed by node and kind; only a different kind or
        // a replaced texture-backed resource can leave one stale. Text and
        // colour changes, which animate every frame, do not.
        let resource = |kind: &NodeKind| {
            #[cfg(target_os = "macos")]
            if matches!(kind, NodeKind::NativeSurface(_)) {
                return true;
            }
            match kind {
                NodeKind::Image(_) | NodeKind::Svg(_) | NodeKind::Canvas(_) => true,
                NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => quad.decoration.is_some(),
                _ => false,
            }
        };
        if std::mem::discriminant(&self.node(id).kind) != std::mem::discriminant(&kind)
            || resource(&self.node(id).kind)
        {
            self.content_revision = self.content_revision.wrapping_add(1);
        }
        let container_layout = |kind: &NodeKind| match kind {
            NodeKind::Container(layout) | NodeKind::Panel { layout, .. } => Some(*layout),
            _ => None,
        };
        let old_layout = container_layout(&self.node(id).kind);
        let layout_unchanged = matches!(
            (&self.node(id).kind, &kind),
            (
                NodeKind::Rect(_) | NodeKind::Quad(_),
                NodeKind::Rect(_) | NodeKind::Quad(_)
            )
        ) || (old_layout.is_some() && old_layout == container_layout(&kind))
            || matches!((&self.node(id).kind, &kind),
                (NodeKind::Text { text: old_text, font_size: old_size, .. },
                 NodeKind::Text { text: new_text, font_size: new_size, .. })
                 if old_text == new_text && old_size == new_size)
            || matches!((&self.node(id).kind, &kind),
                (NodeKind::RichText { text: old }, NodeKind::RichText { text: new }) if old.same_metrics(new))
            || matches!((&self.node(id).kind, &kind),
                (NodeKind::Image(old), NodeKind::Image(new))
                if (old.width() == new.width() && old.height() == new.height())
                    // Both concrete axes make intrinsic source dimensions
                    // irrelevant, including when min/max constraints clamp them.
                    || (self.node(id).style.width.is_some() && self.node(id).style.height.is_some()));
        let layout_unchanged = layout_unchanged
            || matches!(
                (&self.node(id).kind, &kind),
                (NodeKind::Canvas(_), NodeKind::Canvas(_)) | (NodeKind::Svg(_), NodeKind::Svg(_))
            );
        #[cfg(target_os = "macos")]
        let layout_unchanged = layout_unchanged
            || matches!((&self.node(id).kind,&kind),(NodeKind::NativeSurface(old),NodeKind::NativeSurface(new)) if (old.width(),old.height())==(new.width(),new.height()) || (self.node(id).style.width.is_some() && self.node(id).style.height.is_some()));
        self.damage_subtree(id);
        if matches!((&self.node(id).kind, &kind), (NodeKind::Image(old), NodeKind::Image(new))
            if old.transform() != new.transform())
        {
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        if matches!((&self.node(id).kind, &kind), (NodeKind::Svg(old), NodeKind::Svg(new)) if old.transform()!=new.transform())
        {
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        self.node_mut(id).kind = kind;
        self.invalidate(id, PAINT);
        if !layout_unchanged {
            self.invalidate_layout(id);
        }
    }
    pub fn append(&mut self, parent: NodeId, kind: NodeKind, mut style: Style) -> NodeId {
        self.content_revision = self.content_revision.wrapping_add(1);
        self.tree_revision = self.tree_revision.wrapping_add(1);
        if let Some(options) = &style.layout_options {
            let normalized = (**options).normalized();
            if normalized != **options {
                style.layout_options = Some(Arc::new(normalized));
            }
        }
        self.node(parent);
        let node = Node {
            cursor: None,
            parent: Some(parent),
            children: Vec::new(),
            kind,
            font: Default::default(),
            style,
            bounds: Rect::default(),
            transform: Transform::default(),
            paint_transform: Affine::IDENTITY,
            paint_origin: [0., 0.],
            projection_transform: Affine::IDENTITY,
            effects: Effects::default(),
            dirty: LAYOUT | PAINT,
            queued: true,
            measurement: None,
            alternates: Vec::new(),
            ink: std::cell::Cell::new(None),
            ink_valid: std::cell::Cell::new(false),
            text_size: TextSizes::default(),
            advanced_layout: None,
            arrange_dirty: true,
            isolated: false,
            scroll_copy: false,
            layer_revision: 0,
            paint_revision: 0,
        };
        let id = if let Some(index) = self.free.pop() {
            self.slots[index as usize].node = Some(node);
            NodeId {
                index,
                generation: self.slots[index as usize].generation,
            }
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                node: Some(node),
            });
            NodeId {
                index,
                generation: 0,
            }
        };
        self.node_mut(parent).children.push(id);
        self.dirty.push(id);
        self.invalidate_layout(parent);
        id
    }
    pub fn remove(&mut self, id: NodeId) {
        self.content_revision = self.content_revision.wrapping_add(1);
        self.tree_revision = self.tree_revision.wrapping_add(1);
        assert_ne!(id, self.root, "cannot remove scene root");
        self.damage_subtree(id);
        let parent = self.node(id).parent.unwrap();
        self.node_mut(parent).children.retain(|child| *child != id);
        self.free_subtree(id);
        self.invalidate_layout(parent);
        // Hidden windows may retain damage for an arbitrarily long time while
        // virtual rows are replaced. Amortize stale-generation cleanup so that
        // the queue stays proportional to arena capacity, not update count.
        if self.dirty.len() > self.slots.len().saturating_mul(2).max(64) {
            let slots = &self.slots;
            self.dirty.retain(|id| {
                slots
                    .get(id.index as usize)
                    .is_some_and(|slot| slot.generation == id.generation && slot.node.is_some())
            });
        }
    }
    fn free_subtree(&mut self, id: NodeId) {
        let node = self.slots[id.index as usize].node.take().unwrap();
        self.cursor_styles -= usize::from(node.cursor.is_some());
        if node.isolated {
            self.isolation_count -= 1;
        }
        if node.scroll_copy {
            self.scroll_copy_count -= 1;
        }
        if node.effects.blur_radius > 0.0 {
            self.blur_nodes.retain(|entry| *entry != id);
        }
        for child in node.children {
            self.free_subtree(child);
        }
        self.slots[id.index as usize].generation = self.slots[id.index as usize]
            .generation
            .checked_add(1)
            .expect("node generation exhausted");
        self.free.push(id.index);
    }
    pub fn set_text(&mut self, id: NodeId, value: impl Into<Arc<str>>) {
        let value = value.into();
        let node = self.node(id);
        let NodeKind::Text { text, .. } = &node.kind else {
            panic!("set_text requires a text node")
        };
        if *text == value {
            return;
        }
        let fixed = node.style.width.is_some() && node.style.height.is_some();
        self.damage_subtree(id);
        let node = self.node_mut(id);
        if let NodeKind::Text { text, .. } = &mut node.kind {
            *text = value;
        }
        // Sizes of the old text; a fixed box skips relayout, not these.
        node.text_size = TextSizes::default();
        self.invalidate(id, PAINT);
        if !fixed {
            self.invalidate_layout(id);
        }
    }
    pub fn set_style(&mut self, id: NodeId, mut style: Style) {
        if let Some(options) = &style.layout_options
            && !self
                .node(id)
                .style
                .layout_options
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, options))
        {
            let normalized = (**options).normalized();
            if normalized != **options {
                style.layout_options = Some(Arc::new(normalized));
            }
        }
        if self.node(id).style == style {
            return;
        }
        self.damage_subtree(id);
        if own_visible(&self.node(id).style) != own_visible(&style) {
            self.visibility_revision = self.visibility_revision.wrapping_add(1);
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        self.node_mut(id).style = style;
        self.damage_subtree(id);
        self.invalidate(id, PAINT);
        self.invalidate_layout(id);
    }
    pub fn resize(&mut self, width: f32, height: f32) {
        if self.viewport.width == width && self.viewport.height == height {
            return;
        }
        self.viewport = Rect::new(0.0, 0.0, width, height);
        let mut style = self.node(self.root).style.clone();
        style.width = Some(width);
        style.height = Some(height);
        self.set_style(self.root, style);
        self.add_damage(self.viewport);
    }
    /// Set a translation, normalizing each non-finite coordinate to zero.
    pub fn set_transform(&mut self, id: NodeId, mut transform: Transform) {
        let coordinate = |value: f32| {
            if value.is_finite() && value != 0.0 {
                value
            } else {
                0.0
            }
        };
        transform.x = coordinate(transform.x);
        transform.y = coordinate(transform.y);
        if self.node(id).transform == transform {
            return;
        }
        if self.node(id).scroll_copy {
            // Resolved at flush: a copy plus the exposed strip, or full damage.
            match self.pending_moves.iter_mut().find(|(node, ..)| *node == id) {
                Some((.., again)) => *again = true,
                None => {
                    let from = self.node(id).transform;
                    self.pending_moves.push((id, from, false));
                }
            }
            self.node_mut(id).transform = transform;
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
            self.invalidate(id, COMPOSITE);
            return;
        }
        self.damage_subtree(id);
        self.node_mut(id).transform = transform;
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        self.damage_subtree(id);
        self.invalidate(id, COMPOSITE);
    }
    /// Set a subtree transform in node-local coordinates. Non-finite matrices
    /// normalize to identity; singular matrices deliberately paint no area.
    pub fn set_paint_transform(&mut self, id: NodeId, matrix: Affine) {
        self.set_paint_transform_origin(id, matrix, [0., 0.]);
    }
    /// A normalized pivot follows the allocation when layout changes.
    pub fn set_paint_transform_origin(&mut self, id: NodeId, matrix: Affine, origin: [f32; 2]) {
        let origin = origin.map(|value| if value.is_finite() { value } else { 0.5 });
        if self.node(id).paint_origin != origin {
            self.damage_subtree(id);
            self.node_mut(id).paint_origin = origin;
            self.invalidate_affine(id);
            self.damage_subtree(id);
        }
        self.set_affine(id, matrix, false);
    }
    pub fn set_projection_transform(&mut self, id: NodeId, matrix: Affine) {
        self.set_affine(id, matrix, true);
    }
    fn set_affine(&mut self, id: NodeId, matrix: Affine, projection: bool) {
        let matrix = if matrix.is_finite() {
            matrix
        } else {
            Affine::IDENTITY
        };
        let old = if projection {
            self.node(id).projection_transform
        } else {
            self.node(id).paint_transform
        };
        if matrix == old {
            return;
        }
        self.damage_subtree(id);
        if projection {
            self.node_mut(id).projection_transform = matrix;
        } else {
            self.node_mut(id).paint_transform = matrix;
        }
        self.invalidate_affine(id);
        self.damage_subtree(id);
    }
    fn invalidate_affine(&mut self, id: NodeId) {
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        // Explicit isolated layers currently cache in world space, including
        // descendants whose axis-aligned coverage can stay unchanged on a flip.
        if self.isolation_count > 0 {
            let isolated: Vec<_> = self
                .isolated_nodes()
                .filter(|node| self.is_within(*node, id))
                .collect();
            for node in isolated {
                self.node_mut(node).layer_revision = self.node(node).layer_revision.wrapping_add(1);
            }
        }
        self.raster_revision = self.raster_revision.wrapping_add(1);
        self.invalidate(id, COMPOSITE);
    }
    /// Apply finite render effects. NaN opacity uses the opaque default;
    /// infinite opacity clamps to its endpoint. Non-finite filter radii disable
    /// their filter, and negative radii clamp to zero.
    pub fn set_effects(&mut self, id: NodeId, mut effects: Effects) {
        effects.opacity = if effects.opacity.is_nan() {
            1.0
        } else {
            effects.opacity.clamp(0.0, 1.0).max(0.0)
        };
        let radius = |value: f32| {
            if value.is_finite() && value > 0.0 {
                value
            } else {
                0.0
            }
        };
        effects.blur_radius = radius(effects.blur_radius);
        effects.edge_fade = radius(effects.edge_fade);
        if self.node(id).effects == effects {
            return;
        }
        self.raster_revision = self.raster_revision.wrapping_add(1);
        if self.node(id).scroll_copy {
            self.node_mut(id).layer_revision = self.node(id).layer_revision.wrapping_add(1);
        }
        if (self.node(id).effects.opacity <= 0.) != (effects.opacity <= 0.) {
            self.visibility_revision = self.visibility_revision.wrapping_add(1);
            self.geometry_revision = self.geometry_revision.wrapping_add(1);
        }
        let old_blur = self.node(id).effects.blur_radius > 0.0;
        if !old_blur && effects.blur_radius > 0.0 {
            self.blur_nodes.push(id);
        }
        if old_blur && effects.blur_radius == 0.0 {
            self.blur_nodes.retain(|entry| *entry != id);
        }
        self.damage_subtree(id);
        self.node_mut(id).effects = effects;
        self.damage_subtree(id);
        self.invalidate(id, COMPOSITE);
    }
    fn invalidate(&mut self, id: NodeId, flags: u8) {
        if flags & (LAYOUT | PAINT) != 0 {
            self.node_mut(id).paint_revision = self.node(id).paint_revision.wrapping_add(1);
            self.raster_revision = self.raster_revision.wrapping_add(1);
        }
        if self.isolation_count > 0 || self.scroll_copy_count > 0 {
            let mut current = Some(id);
            while let Some(ancestor) = current {
                let node = self.node_mut(ancestor);
                if (node.isolated || node.scroll_copy)
                    && (ancestor != id || flags & (LAYOUT | PAINT) != 0)
                {
                    node.layer_revision = node.layer_revision.wrapping_add(1);
                }
                current = node.parent;
            }
        }
        let mut current = Some(id);
        while let Some(ancestor) = current {
            let node = self.node(ancestor);
            if !node.ink_valid.replace(false) {
                break;
            }
            current = node.parent;
        }
        let node = self.node_mut(id);
        node.dirty |= flags;
        if flags & LAYOUT != 0 {
            node.text_size = TextSizes::default();
        }
        if !node.queued {
            node.queued = true;
            self.dirty.push(id);
        }
    }
    /// Mark `id` and the ancestors whose geometry can depend on it. The walk
    /// stops at a layout boundary: its size cannot depend on its content, so
    /// its parent keeps its layout and only the boundary's subtree is redone.
    fn invalidate_layout(&mut self, id: NodeId) {
        self.invalidate(id, LAYOUT);
        let mut child = id;
        while let Some(parent) = self.node(child).parent {
            self.invalidate(parent, LAYOUT);
            if self.is_layout_boundary(parent) {
                self.layout_roots.push(parent);
                return;
            }
            child = parent;
        }
    }
    /// A laid-out container with an explicit width and height: parents measure
    /// it from those (or from their own flex allocation), never from its children.
    fn is_layout_boundary(&self, id: NodeId) -> bool {
        let node = self.node(id);
        matches!(node.kind, NodeKind::Container(_) | NodeKind::Panel { .. })
            && node.parent.is_some()
            && node.style.width.is_some()
            && node.style.height.is_some()
            && node.style.layout_options.is_none()
            && node.measurement.is_some()
    }
    /// Mark every ancestor up to the root, ignoring boundaries.
    fn invalidate_layout_to_root(&mut self, mut id: NodeId) {
        loop {
            self.invalidate(id, LAYOUT);
            match self.node(id).parent {
                Some(parent) => id = parent,
                None => break,
            }
        }
    }
    fn depth(&self, mut id: NodeId) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.node(id).parent {
            depth += 1;
            id = parent;
        }
        depth
    }
    fn add_damage(&mut self, rect: Rect) {
        // Raster coverage rounds fractional edges outwards. Quantize before merging
        // so adjacent subpixel rectangles cannot repaint the same alpha pixel twice,
        // and every primitive covering a touched pixel participates in its repaint.
        let rect = Rect::new(
            rect.x.floor(),
            rect.y.floor(),
            (rect.x + rect.width).ceil() - rect.x.floor(),
            (rect.y + rect.height).ceil() - rect.y.floor(),
        );
        let Some(rect) = rect.intersection(self.viewport) else {
            return;
        };
        merge_damage(&mut self.damage, rect, None);
        // Bound fragmentation: many independent writes are cheaper as one surface update.
        if self.damage.len() > 64 {
            self.damage.clear();
            self.damage.push(self.viewport);
        }
    }
    fn world(&self, id: NodeId) -> (Rect, Effects, Option<Rect>) {
        let mut path: Vec<_> = std::iter::once(id).chain(self.ancestors(id)).collect();
        path.reverse();
        let mut matrix = Affine::IDENTITY;
        let mut effects = self.node(id).effects;
        let mut clip = Some(self.viewport);
        for current in path {
            let ancestor = self.node(current);
            matrix = self.node_matrix(current).then(matrix);
            if !own_visible(&ancestor.style) {
                effects.opacity = 0.;
            }
            if current != id {
                effects.opacity *= ancestor.effects.opacity;
            }
            if current != id || !has_outer_shadow(&ancestor.kind) {
                clip = clipped_affine_bounds(clip, ancestor.bounds, matrix, &ancestor.style);
            }
        }
        (matrix.bounds(self.node(id).bounds), effects, clip)
    }
    fn damage_subtree(&mut self, id: NodeId) {
        if !self.shown(id) {
            return;
        }
        // Resolve the ancestors once, then carry translation and clipping down
        // instead of walking back up from every descendant.
        let (transform, clip) = match self.node(id).parent {
            Some(parent) => self.world_context(parent),
            None => (Affine::IDENTITY, Some(self.viewport)),
        };
        let isolated =
            self.isolation_count > 0 && self.ancestors(id).any(|node| self.node(node).isolated);
        self.damage_subtree_in(id, transform, clip, isolated);
    }
    fn damage_subtree_in(
        &mut self,
        id: NodeId,
        parent: Affine,
        parent_clip: Option<Rect>,
        isolated: bool,
    ) {
        let node = self.node(id);
        // Not laid out since it was appended, so never painted: nothing to
        // repaint yet (a mounting row styles each node before its first
        // layout, which damages where it lands). Nor are its descendants.
        let unplaced = node.measurement.is_none() && node.bounds == Rect::default();
        if !own_visible(&node.style) || unplaced {
            return;
        }
        let transform = self.node_matrix(id).then(parent);
        let bounds = transform.bounds(node.bounds);
        let clip = clipped_affine_bounds(parent_clip, node.bounds, transform, &node.style);
        let paint_clip = if has_outer_shadow(&node.kind) {
            parent_clip
        } else {
            clip
        };
        // Transform ink before clipping: rotated shadows and glyph overhang can
        // exceed the transformed allocation's axis-aligned box.
        let isolated = isolated || self.node(id).isolated;
        let ink = self.own_ink(id).map(|ink| {
            let ink = transform.bounds(ink);
            if isolated { ink.expand(1.) } else { ink }
        });
        if let Some(ink) =
            ink.and_then(|ink| paint_clip.map_or(Some(ink), |clip| ink.intersection(clip)))
        {
            self.add_damage(ink);
        }
        if self.node(id).effects.blur_radius > 0. {
            self.add_damage(bounds);
        }
        for index in 0..self.node(id).children.len() {
            let child = self.node(id).children[index];
            self.damage_subtree_in(child, transform, clip, isolated);
        }
    }
    /// World translation of `id` including its own, and the clip its children
    /// paint within.
    fn world_context(&self, id: NodeId) -> (Affine, Option<Rect>) {
        let matrix = self.world_paint_transform(id);
        let clip = clipped_affine_bounds(
            self.world(id).2,
            self.node(id).bounds,
            matrix,
            &self.node(id).style,
        );
        (matrix, clip)
    }
    /// See `Node::ink`. Computed on demand and cached until invalidated.
    fn ink(&self, id: NodeId) -> Option<Rect> {
        let node = self.node(id);
        if node.ink_valid.get() {
            return node.ink.get();
        }
        if node.effects.opacity <= 0.0 || !own_visible(&node.style) {
            node.ink.set(None);
            node.ink_valid.set(true);
            return None;
        }
        let mut ink = self.own_ink(id);
        for child in &node.children {
            if let Some(child) = self.ink(*child) {
                ink = Some(ink.map_or(child, |ink| ink.union(child)));
            }
        }
        let ink = ink
            .and_then(|ink| {
                let clipped = clipped_bounds(Some(ink), node.bounds, &node.style);
                if has_outer_shadow(&node.kind) {
                    self.own_ink(id)
                        .map(|own| clipped.map_or(own, |child| own.union(child)))
                } else {
                    clipped
                }
            })
            .map(|ink| self.node_matrix(id).bounds(ink));
        node.ink.set(ink);
        node.ink_valid.set(true);
        ink
    }
    /// Pixels this node itself can touch, matching what damage covers.
    fn own_ink(&self, id: NodeId) -> Option<Rect> {
        let node = self.node(id);
        let blur = node.effects.blur_radius;
        if matches!(node.kind, NodeKind::Container(_)) && blur <= 0.0 {
            return None;
        }
        let bounds = node.bounds;
        Some(match &node.kind {
            NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => quad.paint_bounds(bounds),
            NodeKind::Image(image) => image.paint_bounds(bounds),
            NodeKind::Svg(svg) => svg.paint_bounds(bounds),
            NodeKind::Text { .. } | NodeKind::RichText { .. } => self.text_ink(id, bounds),
            _ => bounds,
        })
    }
    /// Where a text node's glyphs can land: its lines as laid out at its
    /// width, padded for glyph overhang, rather than its whole box. Text that
    /// changes every frame (a stream in a larger box) repaints only that. Text
    /// laid out otherwise (line clamps, ellipses, padding) or right to left
    /// keeps its box.
    fn text_ink(&self, id: NodeId, bounds: Rect) -> Rect {
        let node = self.node(id);
        let style = &node.style;
        let padded = style.padding != 0.
            || style
                .padding_edges
                .is_some_and(|edges| edges != Insets::default());
        if style.text_options != Default::default() || padded {
            return bounds;
        }
        // Renderers lay text out at its box's width, wrapping or not.
        let limit = Some(bounds.width);
        // What layout measured serves when it breaks lines as `limit` would:
        // a size measured wider (or unwrapped) whose widest line fits.
        let revision = self.text_shaper_revision;
        let cached = node
            .text_size
            .0
            .iter()
            .flatten()
            .find_map(|(measured, at, size)| {
                let same = *at == revision
                    && match (limit, measured) {
                        (None, None) => true,
                        (Some(width), None) => size.0 <= width,
                        (Some(width), Some(measured)) => *measured >= width && size.0 <= width,
                        (None, Some(_)) => false,
                    };
                same.then_some(*size)
            });
        let rtl = |text: &str| {
            text.chars().any(|c| {
                matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
            })
        };
        let (size, font_size, line, align) = match &node.kind {
            NodeKind::Text {
                text, font_size, ..
            } => {
                // Headless approximations are not what a renderer draws.
                if rtl(text) || (self.font_measurer.is_none() && self.font_shaper.is_none()) {
                    return bounds;
                }
                let size = cached.unwrap_or_else(|| match &self.font_measurer {
                    Some(measure) => measure(text, *font_size, limit, &node.font),
                    None => self.measure_text_with_font(text, *font_size, limit, &node.font),
                });
                let line = node.font.line_height.resolve(*font_size);
                (size, *font_size, line, node.font.align)
            }
            NodeKind::RichText { text } => {
                if text.options() != Default::default()
                    || rtl(text.text())
                    || !self.rich_measurer.is_native()
                {
                    return bounds;
                }
                let size =
                    cached.unwrap_or_else(|| self.rich_measurer.measure_mounted(text, limit));
                let font_size = text
                    .runs()
                    .iter()
                    .map(|run| run.font_size)
                    .fold(0., f32::max);
                let line = text
                    .runs()
                    .iter()
                    .map(|run| run.font.line_height.resolve(run.font_size))
                    .fold(0., f32::max);
                let align = text
                    .runs()
                    .first()
                    .map_or_else(Default::default, |run| run.font.align);
                (size, font_size, line, align)
            }
            _ => return bounds,
        };
        // Lines starting inside the box are drawn whole.
        let (width, height) = (size.0, size.1.min(bounds.height + line));
        if !(width.is_finite() && height.is_finite()) {
            return bounds;
        }
        let x = match align {
            crate::text_layout::TextAlign::Start | crate::text_layout::TextAlign::Left => bounds.x,
            crate::text_layout::TextAlign::Center => bounds.x + (bounds.width - width) / 2.,
            crate::text_layout::TextAlign::Right => bounds.x + bounds.width - width,
        };
        // Italic and antialiased edges reach past advances and line boxes.
        let pad = (font_size * 0.3).max(2.);
        Rect::new(x - pad, bounds.y - pad, width + 2. * pad, height + 2. * pad)
    }
    /// Whether `id` and its ancestors are displayed: a hidden node paints
    /// nothing, so its changes damage nothing. Hiding one damages what it
    /// painted before the change.
    fn shown(&self, id: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            let node = self.node(node);
            if !own_visible(&node.style) {
                return false;
            }
            current = node.parent;
        }
        true
    }
    fn damage_node(&mut self, id: NodeId) {
        if !self.shown(id) {
            return;
        }
        let (bounds, effects, clip) = self.world(id);
        self.damage_painted(id, bounds, effects.blur_radius, clip);
    }
    fn damage_painted(&mut self, id: NodeId, bounds: Rect, blur_radius: f32, clip: Option<Rect>) {
        let isolated = self.isolation_count > 0
            && std::iter::once(id)
                .chain(self.ancestors(id))
                .any(|node| self.node(node).isolated);
        let ink = self.own_ink(id).map(|ink| {
            let ink = self.world_paint_transform(id).bounds(ink);
            if isolated { ink.expand(1.) } else { ink }
        });
        let ink = if blur_radius > 0. {
            Some(ink.map_or(bounds, |ink| ink.union(bounds)))
        } else {
            ink
        };
        if let Some(bounds) =
            ink.and_then(|ink| clip.map_or(Some(ink), |clip| ink.intersection(clip)))
        {
            self.add_damage(bounds);
        }
    }
    fn measure(&mut self, id: NodeId, mut constraints: Constraints) -> (f32, f32) {
        let node = self.node(id);
        constraints.hidden |= node.style.layout_options.as_deref()
            .is_some_and(|options| options.display == Some(crate::layout::Display::None));
        let pass = self.layout_pass;
        let usable = |m: &Measurement| {
            m.constraints == constraints && (node.dirty & LAYOUT == 0 || m.pass == pass)
        };
        if let Some(current) = &node.measurement
            && usable(current)
        {
            return current.size;
        }
        if let Some(index) = node.alternates.iter().position(usable) {
            // Promote it: `arrange` places children from the current result.
            let node = self.node_mut(id);
            let hit = node.alternates.remove(index);
            let size = hit.size;
            if let Some(previous) = node.measurement.replace(hit) {
                node.alternates.insert(0, previous);
            }
            node.arrange_dirty = true;
            return size;
        }
        if constraints.hidden {
            return self.measure_hidden(id, constraints);
        }
        let node = self.node(id);
        if let NodeKind::Container(layout) | NodeKind::Panel { layout, .. } = node.kind
            && self.uses_advanced_layout(id)
        {
            return self.measure_advanced(id, layout, constraints);
        }
        self.node_mut(id).advanced_layout = None;
        let style = self.resolved_leaf_style(id, constraints);
        let padding = style.padding_edges.unwrap_or(Insets::all(style.padding));
        let padding = Insets {
            top: padding.top.max(0.0),
            right: padding.right.max(0.0),
            bottom: padding.bottom.max(0.0),
            left: padding.left.max(0.0),
        };
        let horizontal_padding = padding.left + padding.right;
        let vertical_padding = padding.top + padding.bottom;
        let explicit_w = constraints
            .force_width
            .or(style.width)
            .or_else(|| percentage(style.width_percent, constraints.percent_width))
            .map(|w| dimension(w, style.min_width, style.max_width));
        let explicit_h = constraints
            .force_height
            .or(style.height)
            .or_else(|| percentage(style.height_percent, constraints.percent_height))
            .map(|h| dimension(h, style.min_height, style.max_height));
        let available_w = explicit_w
            .or_else(|| {
                constraints
                    .width
                    .map(|w| w.min(style.max_width.unwrap_or(f32::INFINITY)))
            })
            .or(style.max_width)
            .map(|w| (w - horizontal_padding).max(0.0));
        let available_h = explicit_h
            .or_else(|| {
                constraints
                    .height
                    .map(|h| h.min(style.max_height.unwrap_or(f32::INFINITY)))
            })
            .or(style.max_height)
            .map(|h| (h - vertical_padding).max(0.0));
        let percent_width = explicit_w.map(|w| (w - horizontal_padding).max(0.0));
        let percent_height = explicit_h.map(|h| (h - vertical_padding).max(0.0));
        let kind = self.node(id).kind.clone();
        let mut children = Vec::new();
        let (mut width, mut height) = match kind {
            NodeKind::Text {
                text, font_size, ..
            } => {
                let limit = (style.text_wrap || style.text_options != Default::default())
                    .then_some(available_w)
                    .flatten();
                let revision = self.text_shaper_revision;
                if let Some(size) = self.node(id).text_size.get(limit, revision) {
                    size
                } else {
                    let size = if style.text_options != Default::default() {
                        let runs = if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![crate::rich_text::TextRun {
                                range: 0..text.len(),
                                font: self.node(id).font.clone(),
                                font_size,
                                ..Default::default()
                            }]
                        };
                        let rich = crate::rich_text::RichText::new(text, runs)
                            .unwrap()
                            .with_options(style.text_options);
                        self.rich_measurer.measure_mounted(&rich, limit)
                    } else {
                        let font = &self.node(id).font;
                        match &self.font_measurer {
                            Some(measure) => measure(&text, font_size, limit, font),
                            None => self.measure_text_with_font(&text, font_size, limit, font),
                        }
                    };
                    self.node_mut(id).text_size.insert(limit, revision, size);
                    size
                }
            }
            NodeKind::RichText { text } => {
                let limit = (style.text_wrap || text.options() != Default::default())
                    .then_some(available_w)
                    .flatten();
                // Same cache as plain text: colour-only changes keep layout
                // (see `set_kind`), so they keep this size too.
                let revision = self.text_shaper_revision;
                if let Some(size) = self.node(id).text_size.get(limit, revision) {
                    size
                } else {
                    let size = self.rich_measurer.measure_mounted(&text, limit);
                    self.node_mut(id).text_size.insert(limit, revision, size);
                    size
                }
            }
            NodeKind::Rect(_) | NodeKind::Quad(_) | NodeKind::Canvas(_) => (0.0, 0.0),
            NodeKind::Svg(_) => (24., 24.),
            NodeKind::Image(image) => (image.width() as f32, image.height() as f32),
            #[cfg(target_os = "macos")]
            NodeKind::NativeSurface(frame) => (frame.width() as f32, frame.height() as f32),
            NodeKind::Container(layout) | NodeKind::Panel { layout, .. } => {
                let (absolute, ids): (Vec<_>, Vec<_>) = self
                    .node(id)
                    .children
                    .iter()
                    .copied()
                    .partition(|child| self.node(*child).style.absolute);
                let mut sizes: Vec<_> = ids
                    .iter()
                    .map(|child| {
                        self.measure(
                            *child,
                            Constraints {
                                width: available_w,
                                height: available_h,
                                percent_width,
                                percent_height,
                                ..Constraints::default()
                            },
                        )
                    })
                    .collect();
                let horizontal = layout == Layout::Row;
                let main_margin = |s: &Style| {
                    if horizontal {
                        s.margin.left + s.margin.right
                    } else {
                        s.margin.top + s.margin.bottom
                    }
                };
                let main = |size: (f32, f32)| if horizontal { size.0 } else { size.1 };
                let gaps = style.gap * ids.len().saturating_sub(1) as f32;
                let natural_main = sizes
                    .iter()
                    .enumerate()
                    .map(|(i, size)| main(*size) + main_margin(&self.node(ids[i]).style))
                    .sum::<f32>()
                    + gaps;
                let main_limit = if horizontal {
                    Some(
                        dimension(
                            explicit_w.unwrap_or(natural_main + horizontal_padding),
                            style.min_width,
                            style.max_width,
                        ) - horizontal_padding,
                    )
                } else {
                    Some(
                        dimension(
                            explicit_h.unwrap_or(natural_main + vertical_padding),
                            style.min_height,
                            style.max_height,
                        ) - vertical_padding,
                    )
                };
                if layout != Layout::Overlay
                    && let Some(limit) = main_limit
                {
                    let bases: Vec<_> = sizes.iter().copied().map(main).collect();
                    let mut frozen = vec![false; ids.len()];
                    for _ in 0..=ids.len() {
                        let used: f32 = sizes
                            .iter()
                            .enumerate()
                            .map(|(i, size)| main(*size) + main_margin(&self.node(ids[i]).style))
                            .sum::<f32>()
                            + gaps;
                        let remaining = limit - used;
                        if remaining.abs() < 0.001 {
                            break;
                        }
                        let weights: Vec<_> = ids
                            .iter()
                            .enumerate()
                            .map(|(i, child)| {
                                let s = &self.node(*child).style;
                                if frozen[i] {
                                    0.0
                                } else if remaining > 0.0 {
                                    s.flex_grow.max(0.0)
                                } else {
                                    s.flex_shrink.max(0.0) * bases[i]
                                }
                            })
                            .collect();
                        let weight: f32 = weights.iter().sum();
                        if weight <= 0.0 {
                            break;
                        }
                        let mut clamped = false;
                        for (i, child) in ids.iter().enumerate() {
                            if weights[i] == 0.0 {
                                continue;
                            }
                            let s = &self.node(*child).style;
                            let wanted = main(sizes[i]) + remaining * weights[i] / weight;
                            let allocated = if horizontal {
                                dimension(wanted, s.min_width, s.max_width)
                            } else {
                                dimension(wanted, s.min_height, s.max_height)
                            };
                            if (allocated - wanted).abs() > 0.001 {
                                frozen[i] = true;
                                clamped = true;
                            }
                            sizes[i] = self.measure(
                                *child,
                                Constraints {
                                    width: available_w,
                                    height: available_h,
                                    percent_width,
                                    percent_height,
                                    force_width: horizontal.then_some(allocated),
                                    force_height: (!horizontal).then_some(allocated),
                                    hidden: false,
                                },
                            );
                        }
                        if !clamped {
                            break;
                        }
                    }
                }
                // A flex item receives a definite main allocation even when
                // its natural size already consumes exactly the available space.
                // Its percentage descendants must see that allocation as a basis.
                if layout != Layout::Overlay
                    && if horizontal {
                        explicit_w.is_some()
                    } else {
                        explicit_h.is_some()
                    }
                {
                    for (i, child) in ids.iter().enumerate() {
                        let child_style = &self.node(*child).style;
                        if child_style.flex_grow > 0.0 || child_style.flex_shrink > 0.0 {
                            sizes[i] = self.measure(
                                *child,
                                Constraints {
                                    width: available_w,
                                    height: available_h,
                                    percent_width,
                                    percent_height,
                                    force_width: horizontal.then_some(sizes[i].0),
                                    force_height: (!horizontal).then_some(sizes[i].1),
                                    hidden: false,
                                },
                            );
                        }
                    }
                }
                let mut natural_w = 0.0_f32;
                let mut natural_h = 0.0_f32;
                for (i, child) in ids.iter().enumerate() {
                    let m = self.node(*child).style.margin;
                    let (w, h) = (sizes[i].0 + m.left + m.right, sizes[i].1 + m.top + m.bottom);
                    match layout {
                        Layout::Row => {
                            natural_w += w;
                            natural_h = natural_h.max(h);
                        }
                        Layout::Column => {
                            natural_h += h;
                            natural_w = natural_w.max(w);
                        }
                        Layout::Overlay => {
                            natural_w = natural_w.max(w);
                            natural_h = natural_h.max(h);
                        }
                    }
                }
                if layout == Layout::Row {
                    natural_w += gaps;
                }
                if layout == Layout::Column {
                    natural_h += gaps;
                }
                let inner_w = dimension(
                    explicit_w.unwrap_or(natural_w + horizontal_padding),
                    style.min_width,
                    style.max_width,
                ) - horizontal_padding;
                let inner_h = dimension(
                    explicit_h.unwrap_or(natural_h + vertical_padding),
                    style.min_height,
                    style.max_height,
                ) - vertical_padding;
                let extra = if horizontal {
                    inner_w - natural_w
                } else {
                    inner_h - natural_h
                };
                let (mut cursor, extra_gap) = match style.justify {
                    Justify::Start => (0.0, 0.0),
                    Justify::Center => (extra / 2.0, 0.0),
                    Justify::End => (extra, 0.0),
                    Justify::SpaceBetween if ids.len() > 1 => {
                        (0.0, extra.max(0.0) / (ids.len() - 1) as f32)
                    }
                    Justify::SpaceAround if !ids.is_empty() => (
                        extra.max(0.0) / ids.len() as f32 / 2.0,
                        extra.max(0.0) / ids.len() as f32,
                    ),
                    Justify::SpaceEvenly => {
                        let gap = extra.max(0.0) / (ids.len() + 1) as f32;
                        (gap, gap)
                    }
                    _ => (0.0, 0.0),
                };
                for (i, child) in ids.iter().enumerate() {
                    let s = self.node(*child).style.clone();
                    let m = s.margin;
                    if style.align == Align::Stretch {
                        let stretch_w = layout != Layout::Row
                            && s.width.is_none()
                            && percentage(s.width_percent, percent_width).is_none();
                        let stretch_h = layout != Layout::Column
                            && s.height.is_none()
                            && percentage(s.height_percent, percent_height).is_none();
                        if stretch_w || stretch_h {
                            sizes[i] = self.measure(
                                *child,
                                Constraints {
                                    width: available_w,
                                    height: available_h,
                                    percent_width,
                                    percent_height,
                                    force_width: if stretch_w {
                                        Some((inner_w - m.left - m.right).max(0.0))
                                    } else if horizontal {
                                        Some(sizes[i].0)
                                    } else {
                                        None
                                    },
                                    force_height: if stretch_h {
                                        Some((inner_h - m.top - m.bottom).max(0.0))
                                    } else if !horizontal {
                                        Some(sizes[i].1)
                                    } else {
                                        None
                                    },
                                    hidden: false,
                                },
                            );
                        }
                    }
                    let (w, h) = sizes[i];
                    let cross_free = if horizontal {
                        inner_h - h - m.top - m.bottom
                    } else {
                        inner_w - w - m.left - m.right
                    };
                    let cross = match style.align {
                        Align::Center => cross_free / 2.0,
                        Align::End => cross_free,
                        _ => 0.0,
                    };
                    let (cx, cy) = match layout {
                        Layout::Row => (cursor + m.left, cross + m.top),
                        Layout::Column => (cross + m.left, cursor + m.top),
                        Layout::Overlay => {
                            let free = inner_h - h - m.top - m.bottom;
                            let y = match style.justify {
                                Justify::Center => free / 2.,
                                Justify::End => free,
                                _ => 0.,
                            };
                            (cross + m.left, y + m.top)
                        }
                    };
                    children.push((*child, Rect::new(cx + padding.left, cy + padding.top, w, h)));
                    cursor += main(sizes[i]) + main_margin(&s) + style.gap + extra_gap;
                }
                for child in absolute {
                    let margin = self.node(child).style.margin;
                    let (w, h) = self.measure(
                        child,
                        Constraints {
                            width: Some((inner_w - margin.left - margin.right).max(0.0)),
                            height: Some((inner_h - margin.top - margin.bottom).max(0.0)),
                            percent_width: Some(inner_w.max(0.0)),
                            percent_height: Some(inner_h.max(0.0)),
                            ..Constraints::default()
                        },
                    );
                    children.push((
                        child,
                        Rect::new(padding.left + margin.left, padding.top + margin.top, w, h),
                    ));
                }
                (natural_w, natural_h)
            }
        };
        width = dimension(
            explicit_w.unwrap_or(width + horizontal_padding),
            style.min_width,
            style.max_width,
        );
        height = dimension(
            explicit_h.unwrap_or(height + vertical_padding),
            style.min_height,
            style.max_height,
        );
        self.store_measurement(id, constraints, (width, height), children);
        (width, height)
    }
    /// Make a new result current, keeping earlier ones that are still valid.
    fn store_measurement(
        &mut self,
        id: NodeId,
        constraints: Constraints,
        size: (f32, f32),
        children: Vec<(NodeId, Rect)>,
    ) {
        let pass = self.layout_pass;
        let child_constraints = children
            .iter()
            .map(|(child, _)| {
                self.node(*child)
                    .measurement
                    .as_ref()
                    .map(|m| m.constraints)
            })
            .collect();
        let node = self.node_mut(id);
        let dirty = node.dirty & LAYOUT != 0;
        if let Some(previous) = node.measurement.take() {
            node.alternates.insert(0, previous);
        }
        // Results from before a node changed are stale.
        node.alternates
            .retain(|m| m.constraints != constraints && (!dirty || m.pass == pass));
        node.alternates.truncate(MEASUREMENT_ALTERNATES);
        node.arrange_dirty = true;
        node.measurement = Some(Measurement {
            constraints,
            size,
            children,
            child_constraints,
            pass,
        });
    }
    /// `covered`: an ancestor moved and already damaged this whole subtree at
    /// its old and new positions.
    fn arrange(&mut self, id: NodeId, x: f32, y: f32, report: &mut FrameReport, covered: bool) {
        let cache = self.node(id).measurement.as_ref().unwrap();
        let bounds = Rect::new(x, y, cache.size.0, cache.size.1);
        if !self.node(id).arrange_dirty && self.node(id).bounds == bounds {
            return;
        }
        let children = cache.children.clone();
        let child_constraints = cache.child_constraints.clone();
        let changed = self.node(id).bounds != bounds;
        if changed || self.node(id).arrange_dirty {
            report.layout_nodes += 1;
        }
        let damage = changed && !covered;
        if damage {
            self.damage_subtree(id);
        }
        if changed {
            self.node_mut(id).bounds = bounds;
            self.invalidate(id, PAINT);
        }
        for ((child, rect), constraints) in children.into_iter().zip(child_constraints) {
            // Cached: promotes the matching result; evicted: re-measures.
            if let Some(constraints) = constraints {
                self.measure(child, constraints);
            }
            self.arrange(child, x + rect.x, y + rect.y, report, covered || changed);
        }
        if damage {
            self.damage_subtree(id);
        }
        self.node_mut(id).dirty &= !LAYOUT;
        self.node_mut(id).arrange_dirty = false;
    }
    pub(crate) fn geometry_revision(&self) -> u64 {
        self.geometry_revision
    }
    pub(crate) fn layout_revision(&self) -> u64 {
        self.layout_revision
    }
    /// Updates geometry for hit testing without consuming pending paint damage.
    /// The next `flush` reports this work and all damage accumulated by layout.
    pub fn prepare_layout(&mut self) {
        if self.node(self.root).dirty & LAYOUT == 0 && self.layout_roots.is_empty() {
            return;
        }
        self.layout_pass = self.layout_pass.wrapping_add(1);
        let mut report = FrameReport::default();
        if self.node(self.root).dirty & LAYOUT != 0 {
            self.measure(self.root, Constraints::default());
            self.arrange(self.root, 0.0, 0.0, &mut report, false);
        }
        // Boundaries under clean ancestors keep their place and size; lay out
        // just their subtrees, outermost first.
        let mut roots = std::mem::take(&mut self.layout_roots);
        roots.retain(|id| self.contains(*id));
        roots.sort_by_key(|id| (self.depth(*id), id.index, id.generation));
        roots.dedup();
        let mut relayout_all = false;
        for id in roots {
            if !self.contains(id) || self.node(id).dirty & LAYOUT == 0 {
                continue;
            }
            let Some((constraints, size)) = self
                .node(id)
                .measurement
                .as_ref()
                .map(|m| (m.constraints, m.size))
            else {
                relayout_all = true;
                self.invalidate_layout_to_root(id);
                continue;
            };
            if self.measure(id, constraints) != size {
                // Not actually independent of its content: lay out normally.
                relayout_all = true;
                self.invalidate_layout_to_root(id);
                continue;
            }
            let bounds = self.node(id).bounds;
            self.arrange(id, bounds.x, bounds.y, &mut report, false);
        }
        if relayout_all && self.node(self.root).dirty & LAYOUT != 0 {
            self.measure(self.root, Constraints::default());
            self.arrange(self.root, 0.0, 0.0, &mut report, false);
        }
        self.pending_layout_nodes += report.layout_nodes;
        self.layout_revision = self.layout_revision.wrapping_add(1);
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
    }
    pub fn flush(&mut self) -> FrameReport {
        let mut report = FrameReport::default();
        if self.dirty.is_empty() {
            return report;
        }
        self.prepare_layout();
        report.layout_nodes = std::mem::take(&mut self.pending_layout_nodes);
        let dirty = std::mem::take(&mut self.dirty);
        for id in &dirty {
            if !self.contains(*id) {
                continue;
            }
            let flags = self.node(*id).dirty;
            if flags & PAINT != 0 {
                report.paint_nodes += 1;
                self.damage_node(*id);
            }
            if flags & COMPOSITE != 0 {
                report.composite_nodes += 1;
            }
            let node = self.node_mut(*id);
            node.dirty = 0;
            node.queued = false;
        }
        self.dirty = dirty;
        self.dirty.clear();
        // Backdrop-filter output depends on pixels behind it, even when its own node is clean.
        if !self.damage.is_empty() && !self.blur_nodes.is_empty() {
            let mut blur_regions: Vec<(Rect, bool)> = self
                .blur_nodes
                .iter()
                .filter_map(|id| {
                    let (bounds, effects, clip) = self.world(*id);
                    // Fully transparent filters cannot change the visible backdrop.
                    // Hide/show still damages both old and new subtree bounds.
                    if effects.opacity <= 0. {
                        return None;
                    }
                    bounds
                        .intersection(clip.unwrap_or(self.viewport))
                        .map(|r| (r, false))
                })
                .collect();
            loop {
                let mut changed = false;
                for (rect, processed) in &mut blur_regions {
                    if !*processed && self.damage.iter().any(|damage| damage.intersects(*rect)) {
                        self.add_damage(*rect);
                        *processed = true;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
        }
        self.resolve_moves();
        report.damage = std::mem::take(&mut self.damage);
        if !report.damage.is_empty() || !self.moves.is_empty() {
            self.flush_serial += 1;
            for scroll in &mut self.moves {
                scroll.serial = self.flush_serial;
            }
        }
        report
    }
    /// Mark `id` as scroll content: translating it reports a `ScrollMove`
    /// instead of damaging everything it paints.
    pub fn set_scroll_copy(&mut self, id: NodeId, enabled: bool) {
        if self.node(id).scroll_copy == enabled {
            return;
        }
        self.scroll_copy_count =
            self.scroll_copy_count - usize::from(self.node(id).scroll_copy) + usize::from(enabled);
        self.node_mut(id).scroll_copy = enabled;
    }
    /// Moves reported by the most recent flush.
    pub fn scroll_moves(&self) -> &[ScrollMove] {
        &self.moves
    }
    /// Counts flushes that reported damage or moves.
    pub fn flush_serial(&self) -> u64 {
        self.flush_serial
    }
    /// Whether `id` is `ancestor` or inside it.
    pub fn is_within(&self, mut id: NodeId, ancestor: NodeId) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.node(id).parent {
                Some(parent) => id = parent,
                None => return false,
            }
        }
    }
    /// Turn this frame's scroll translations into moves: damage what a copy
    /// cannot supply, or everything the content painted where a copy is
    /// unsafe (isolated ancestors, backdrop filters over the clip, nested or
    /// overlapping moves, jumps past the viewport).
    fn resolve_moves(&mut self) {
        self.moves.clear();
        let pending = std::mem::take(&mut self.pending_moves);
        let mut moves = Vec::new();
        for (id, from, again) in pending {
            if !self.contains(id) {
                continue;
            }
            // Geometry-only scrollers (e.g. a mirrored scrollbar's extent)
            // move no pixels and must not invalidate overlapping real content.
            if self.ink(id).is_none() {
                continue;
            }
            let to = self.node(id).transform;
            let (dx, dy) = (to.x - from.x, to.y - from.y);
            if again {
                // Descendants changed at an intermediate position (a measured
                // list re-anchoring between layout passes, possibly back to
                // where it started) were damaged neither where they were
                // painted nor where they end: repaint everything it shows.
                match self.world(id).2 {
                    Some(clip) => self.add_damage(clip),
                    None => self.add_damage(self.viewport),
                }
                continue;
            }
            if dx == 0. && dy == 0. {
                continue;
            }
            let (_, _, clip) = self.world(id);
            let isolated = {
                let mut current = Some(id);
                let mut found = false;
                while let Some(node) = current {
                    found |= self.node(node).isolated || !self.node_matrix(node).is_translation();
                    current = self.node(node).parent;
                }
                found
            };
            let filtered = |scene: &Self, clip: Rect| {
                scene.blur_nodes.iter().any(|blur| {
                    let (bounds, effects, _) = scene.world(*blur);
                    effects.opacity > 0. && bounds.intersects(clip)
                })
            };
            let usable = clip.filter(|clip| {
                !isolated
                    && dx.abs() < clip.width
                    && dy.abs() < clip.height
                    && !filtered(self, *clip)
            });
            match usable {
                Some(clip) => moves.push(ScrollMove {
                    node: id,
                    clip,
                    dx,
                    dy,
                    serial: 0,
                }),
                None => {
                    // Everything the content painted, at both positions.
                    let now = self.node(id).transform;
                    self.node_mut(id).transform = from;
                    self.damage_subtree(id);
                    self.node_mut(id).transform = now;
                    self.damage_subtree(id);
                }
            }
        }
        // Two-axis scrollers translate wrappers on each axis. If the outer
        // wrapper paints only the inner content through the same clip, this
        // is one rigid diagonal move, not two conflicting copies.
        let mut merged = true;
        while merged {
            merged = false;
            'pairs: for outer in 0..moves.len() {
                for inner in 0..moves.len() {
                    if outer != inner
                        && moves[outer].clip == moves[inner].clip
                        && self.is_within(moves[inner].node, moves[outer].node)
                        && self
                            .layer_items(Some(moves[outer].node))
                            .iter()
                            .all(|item| {
                                self.is_within(item.id, moves[inner].node)
                                    || (!item.isolated && self.own_ink(item.id).is_none())
                            })
                    {
                        moves[inner].dx += moves[outer].dx;
                        moves[inner].dy += moves[outer].dy;
                        moves.remove(outer);
                        merged = true;
                        break 'pairs;
                    }
                }
            }
        }
        // Remaining nested or overlapping moves: repaint instead.
        let overlapping: Vec<usize> = (0..moves.len())
            .filter(|&i| {
                (0..moves.len()).any(|j| {
                    i != j
                        && (moves[i].clip.intersects(moves[j].clip)
                            || self.is_within(moves[i].node, moves[j].node))
                })
            })
            .collect();
        for (index, scroll) in moves.into_iter().enumerate() {
            if overlapping.contains(&index) {
                self.add_damage(scroll.clip);
                continue;
            }
            // Other changes inside the clip also land where the copy moves
            // them; the strip the copy cannot fill is exposed.
            let inside: Vec<Rect> = self
                .damage
                .iter()
                .filter_map(|rect| rect.intersection(scroll.clip))
                .collect();
            for rect in inside {
                let shifted = rect.translated(Transform {
                    x: scroll.dx,
                    y: scroll.dy,
                });
                if let Some(rect) = shifted.intersection(scroll.clip) {
                    self.add_damage(rect);
                }
            }
            for rect in exposed(scroll.clip, scroll.dx, scroll.dy) {
                self.add_damage(rect);
            }
            self.moves.push(scroll);
        }
    }
    /// Isolates this subtree into an offscreen group. Opacity then applies once to
    /// overlapping children. Legacy nodes retain per-primitive inherited opacity.
    pub fn set_isolated(&mut self, id: NodeId, isolated: bool) {
        if self.node(id).isolated == isolated {
            return;
        }
        self.damage_subtree(id);
        if isolated {
            self.isolation_count += 1;
        } else {
            self.isolation_count -= 1;
        }
        self.node_mut(id).isolated = isolated;
        self.invalidate(id, PAINT);
    }
    pub fn is_isolated(&self, id: NodeId) -> bool {
        self.node(id).isolated
    }
    pub fn layer_revision(&self, id: NodeId) -> u64 {
        self.node(id).layer_revision
    }
    /// This node's raster content/layout revision, excluding paint transforms,
    /// opacity and ancestor changes. Useful for retaining local primitive pixels.
    pub fn paint_revision(&self, id: NodeId) -> u64 {
        self.node(id).paint_revision
    }
    pub fn isolated_nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.node
                .as_ref()
                .filter(|node| node.isolated)
                .map(|_| NodeId {
                    index: index as u32,
                    generation: slot.generation,
                })
        })
    }
    /// World-space content bounds, without ancestor clipping, for offscreen allocation.
    pub fn layer_bounds(&self, id: NodeId) -> Rect {
        let mut bounds = self.bounds(id);
        for item in self.layer_items(Some(id)) {
            let r = if item.isolated {
                self.layer_bounds(item.id)
            } else if let NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } = item.kind {
                quad.paint_bounds(item.bounds)
            } else if let NodeKind::Svg(svg) = item.kind {
                svg.paint_bounds(item.bounds)
            } else if let NodeKind::Image(image) = item.kind {
                image.paint_bounds(item.bounds)
            } else {
                item.bounds
            };
            let r = if item.isolated {
                r
            } else {
                item.transform.bounds(r)
            };
            let r = item.clip.map_or(Some(r), |clip| r.intersection(clip));
            if let Some(r) = r {
                bounds = bounds.union(r);
            }
        }
        bounds
    }
    /// Paints a single compositor layer, replacing nested isolation boundaries with
    /// one item. A layer root excludes its own opacity/filter from its cached pixels.
    /// Changes whenever nodes are added, removed or change kind. Renderers can
    /// prune per-node caches only when it moves.
    pub fn content_revision(&self) -> u64 {
        self.content_revision
    }
    /// Accessible membership/order changes; texture replacements and rigid
    /// translations only change pixels and bounds, respectively.
    pub fn projection_revision(&self) -> (u64, u64, u64, u64) {
        (
            self.identity,
            self.tree_revision,
            self.layout_revision,
            self.visibility_revision,
        )
    }
    /// Pixel content/effects/layout revision, excluding rigid translations.
    pub fn raster_revision(&self) -> u64 {
        self.raster_revision
    }
    /// True when any node applies a backdrop blur, whose output depends on
    /// pixels outside its own subtree.
    pub fn has_backdrop_blur(&self) -> bool {
        !self.blur_nodes.is_empty()
    }
    /// Bounded sampling/output regions, without traversing unrelated scene nodes.
    pub fn backdrop_regions(&self) -> impl Iterator<Item = (Rect, f32)> + '_ {
        self.blur_nodes.iter().filter_map(|id| {
            let (bounds, effects, clip) = self.world(*id);
            if effects.opacity <= 0. {
                return None;
            }
            bounds
                .intersection(clip.unwrap_or(self.viewport))
                .map(|area| (area, effects.blur_radius))
        })
    }
    pub fn layer_items(&self, root: Option<NodeId>) -> Vec<PaintItem<'_>> {
        self.layer_items_within(root, None)
    }
    /// Like [`Scene::layer_items`], but skips whole subtrees that paint nothing
    /// inside `damage`. Items that remain may still lie partly outside it.
    pub fn layer_items_within(
        &self,
        root: Option<NodeId>,
        damage: Option<&[Rect]>,
    ) -> Vec<PaintItem<'_>> {
        let reach = damage.map(|damage| {
            damage
                .iter()
                .copied()
                .reduce(Rect::union)
                .unwrap_or_default()
        });
        let touches = |ink: Option<Rect>, transform: Affine| -> bool {
            let (Some(damage), Some(reach)) = (damage, reach) else {
                return true;
            };
            let Some(ink) = ink else {
                return false;
            };
            let ink = transform.bounds(ink);
            // Cached isolated images filter one pixel beyond raster coverage.
            // Expand after world composition so an ancestor's scale cannot
            // magnify this conservative sampling halo.
            let ink = if self.isolation_count > 0 {
                ink.expand(1.)
            } else {
                ink
            };
            ink.intersects(reach) && damage.iter().any(|d| d.intersects(ink))
        };
        let start = root.unwrap_or(self.root);
        if !self.layout_visible(start) {
            return Vec::new();
        }
        let transform = self.node(start).parent.map_or(Affine::IDENTITY, |parent| {
            self.world_paint_transform(parent)
        });
        let clip = if root.is_some() {
            None
        } else {
            Some(self.viewport)
        };
        let mut stack = vec![(
            start,
            transform,
            1.,
            clip,
            None::<FadeMask>,
            std::rc::Rc::<[PaintClip]>::from([]),
            Affine::IDENTITY,
        )];
        let mut result = Vec::new();
        while let Some((
            id,
            parent_transform,
            parent_opacity,
            parent_clip,
            mask,
            regions,
            fade_transform,
        )) = stack.pop()
        {
            let node = self.node(id);
            let matrix = self.node_matrix(id).then(parent_transform);
            let (bounds, transform) = primitive_coordinates(node.bounds, matrix);
            let is_root = root == Some(id);
            let isolated = node.isolated && !is_root;
            let mut effects = if is_root {
                Effects::default()
            } else {
                node.effects
            };
            effects.opacity *= parent_opacity;
            // Fully transparent subtrees contribute no cached pixels or layer
            // bounds. An explicit layer root keeps opacity external above.
            if effects.opacity <= 0.0 || !own_visible(&node.style) {
                continue;
            }
            let clip = clipped_affine_bounds(parent_clip, node.bounds, matrix, &node.style);
            let parent_regions = regions.clone();
            let regions = transformed_clips(regions, node.bounds, matrix, &node.style, &node.kind);
            let (child_mask, child_fade_transform) =
                transformed_fade(mask, fade_transform, clip, node.bounds, matrix, &node.style);
            if !isolated {
                stack.extend(
                    node.children
                        .iter()
                        .rev()
                        .filter(|child| touches(self.ink(**child), matrix))
                        .map(|child| {
                            (
                                *child,
                                matrix,
                                effects.opacity,
                                clip,
                                child_mask,
                                regions.clone(),
                                child_fade_transform,
                            )
                        }),
                );
            }
            result.push(PaintItem {
                isolated,
                id,
                bounds,
                transform,
                clip_regions: if has_outer_shadow(&node.kind)
                    || rounded_clip(&node.style, &node.kind).is_some()
                {
                    parent_regions
                } else {
                    regions
                },
                fade_transform,
                kind: &node.kind,
                font: &node.font,
                text_options: node.style.text_options,
                effects,
                clip: if has_outer_shadow(&node.kind) {
                    parent_clip
                } else {
                    clip
                },
                mask,
            });
        }
        result
    }
    pub fn paint_items(&self) -> impl Iterator<Item = PaintItem<'_>> {
        PaintIter {
            scene: self,
            stack: vec![(
                self.root,
                Affine::IDENTITY,
                1.0,
                self.viewport,
                None,
                std::rc::Rc::from([]),
                Affine::IDENTITY,
            )],
        }
    }
}
/// The mask a node passes to its children: its own when it fades its edges
/// (over its clip), otherwise the one it inherited.
fn has_outer_shadow(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } if !quad.shadows().is_empty())
}
fn fade_mask(inherited: Option<FadeMask>, clip: Option<Rect>, style: &Style) -> Option<FadeMask> {
    match clip {
        Some(clip) if style.fade_edges.iter().any(|band| *band > 0.) => {
            Some((clip, style.fade_edges.map(|band| band.max(0.))))
        }
        _ => inherited,
    }
}
fn clip_axes(style: &Style) -> (bool, bool) {
    let options = style.layout_options.as_deref();
    (
        options.and_then(|o| o.clip_x).unwrap_or(style.clip),
        options.and_then(|o| o.clip_y).unwrap_or(style.clip),
    )
}
fn clipped_bounds(parent: Option<Rect>, bounds: Rect, style: &Style) -> Option<Rect> {
    let (x, y) = clip_axes(style);
    if !x && !y {
        return parent;
    }
    let extent = f32::MAX / 4.;
    let base = parent.unwrap_or(Rect::new(-extent, -extent, extent * 2., extent * 2.));
    let mask = Rect::new(
        if x { bounds.x } else { base.x },
        if y { bounds.y } else { base.y },
        if x { bounds.width } else { base.width },
        if y { bounds.height } else { base.height },
    );
    Some(base.intersection(mask).unwrap_or_default())
}
type PaintState = (
    NodeId,
    Affine,
    f32,
    Rect,
    Option<FadeMask>,
    std::rc::Rc<[PaintClip]>,
    Affine,
);
struct PaintIter<'a> {
    scene: &'a Scene,
    stack: Vec<PaintState>,
}
impl<'a> Iterator for PaintIter<'a> {
    type Item = PaintItem<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let (id, parent_transform, parent_opacity, parent_clip, mask, regions, fade_transform) =
            self.stack.pop()?;
        let node = self.scene.node(id);
        let matrix = self.scene.node_matrix(id).then(parent_transform);
        let (bounds, transform) = primitive_coordinates(node.bounds, matrix);
        let mut effects = node.effects;
        if !own_visible(&node.style) {
            effects.opacity = 0.;
        }
        effects.opacity *= parent_opacity;
        let clip = clipped_affine_bounds(Some(parent_clip), node.bounds, matrix, &node.style)
            .unwrap_or(parent_clip);
        let parent_regions = regions.clone();
        let regions = transformed_clips(regions, node.bounds, matrix, &node.style, &node.kind);
        let (child_mask, child_fade_transform) = transformed_fade(
            mask,
            fade_transform,
            Some(clip),
            node.bounds,
            matrix,
            &node.style,
        );
        self.stack.extend(node.children.iter().rev().map(|child| {
            (
                *child,
                matrix,
                effects.opacity,
                clip,
                child_mask,
                regions.clone(),
                child_fade_transform,
            )
        }));
        Some(PaintItem {
            isolated: false,
            id,
            bounds,
            transform,
            clip_regions: if has_outer_shadow(&node.kind)
                || rounded_clip(&node.style, &node.kind).is_some()
            {
                parent_regions
            } else {
                regions
            },
            fade_transform,
            kind: &node.kind,
            font: &node.font,
            text_options: node.style.text_options,
            effects,
            clip: Some(if has_outer_shadow(&node.kind) {
                parent_clip
            } else {
                clip
            }),
            mask,
        })
    }
}
fn contains_point(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
}
fn primitive_coordinates(bounds: Rect, matrix: Affine) -> (Rect, Affine) {
    if matrix.is_translation() {
        (
            bounds.translated(Transform {
                x: matrix.tx,
                y: matrix.ty,
            }),
            Affine::IDENTITY,
        )
    } else {
        (bounds, matrix)
    }
}
fn rounded_clip(style: &Style, kind: &NodeKind) -> Option<[f32; 4]> {
    if clip_axes(style) != (true, true) {
        return None;
    }
    let quad = match kind {
        NodeKind::Quad(q) | NodeKind::Panel { quad: q, .. } => q,
        _ => return None,
    };
    let corners = quad
        .decoration
        .as_ref()
        .and_then(|d| d.corners)
        .unwrap_or(crate::decoration::Corners::all(quad.radius));
    let radii = [
        corners.top_left,
        corners.top_right,
        corners.bottom_right,
        corners.bottom_left,
    ]
    .map(|r| if r.is_finite() { r.max(0.) } else { 0. });
    radii.iter().any(|r| *r > 0.).then_some(radii)
}
fn transformed_clips(
    inherited: std::rc::Rc<[PaintClip]>,
    bounds: Rect,
    matrix: Affine,
    style: &Style,
    kind: &NodeKind,
) -> std::rc::Rc<[PaintClip]> {
    let axes = clip_axes(style);
    let rounded = rounded_clip(style, kind);
    if axes == (false, false) || matrix.is_translation() && rounded.is_none() {
        return inherited;
    }
    let mut regions = inherited.to_vec();
    if let Some(inverse) = matrix.inverse() {
        regions.push(PaintClip {
            bounds,
            inverse,
            axes: [axes.0, axes.1],
            corners: rounded
                .unwrap_or([0.; 4])
                .map(|r| r.min(bounds.width.min(bounds.height).max(0.) * 0.5)),
        });
    } else {
        regions.push(PaintClip {
            bounds: Rect::default(),
            inverse: Affine::IDENTITY,
            axes: [true, true],
            corners: [0.; 4],
        });
    }
    regions.into()
}
fn clipped_affine_bounds(
    parent: Option<Rect>,
    bounds: Rect,
    matrix: Affine,
    style: &Style,
) -> Option<Rect> {
    let axes = clip_axes(style);
    if !matrix.is_translation() && axes != (true, true) {
        return parent;
    }
    clipped_bounds(parent, matrix.bounds(bounds), style)
}
fn transformed_fade(
    inherited: Option<FadeMask>,
    inverse: Affine,
    clip: Option<Rect>,
    bounds: Rect,
    matrix: Affine,
    style: &Style,
) -> (Option<FadeMask>, Affine) {
    if style.fade_edges.iter().any(|band| *band > 0.) && !matrix.is_translation() {
        (
            Some((bounds, style.fade_edges.map(|band| band.max(0.)))),
            matrix.inverse().unwrap_or(Affine::IDENTITY),
        )
    } else if style.fade_edges.iter().any(|band| *band > 0.) {
        (fade_mask(inherited, clip, style), Affine::IDENTITY)
    } else {
        (inherited, inverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtree_affine_composes_with_translation_and_inverse_input() {
        let mut scene = Scene::new(300., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(40.),
                height: Some(40.),
                ..Default::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(10.),
                height: Some(8.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform(parent, Affine::scale(2., 3.));
        scene.set_transform(parent, Transform { x: 50., y: 40. });
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0);
        assert_eq!(scene.bounds(child), Rect::new(50., 40., 20., 24.));
        let local = scene.world_to_local(child, 58., 52.).unwrap();
        // Inverting a 3x scale rounds 1/3 in f32; compare the mathematical
        // round trip with the same tolerance as the affine inverse tests.
        assert!((local.0 - 4.).abs() < 0.0001 && (local.1 - 4.).abs() < 0.0001);
        assert_eq!(scene.hit_test(58., 52.), Some(child));
        assert!(!scene.hit_test_all(73., 52.).contains(&child));
        let item = scene.paint_items().find(|item| item.id == child).unwrap();
        assert_eq!(item.bounds, scene.layout_bounds(child));
        assert_eq!(item.transform.bounds(item.bounds), scene.bounds(child));
    }
    #[test]
    fn affine_origin_tracks_resizing_and_singular_nodes_do_not_hit() {
        let mut scene = Scene::new(300., 300.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(40.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform_origin(node, Affine::scale(2., 2.), [0.5, 0.5]);
        scene.flush();
        assert_eq!(scene.bounds(node), Rect::new(-20., -10., 80., 40.));
        let mut style = scene.style(node);
        style.width = Some(80.);
        scene.set_style(node, style);
        scene.flush();
        assert_eq!(scene.bounds(node), Rect::new(-40., -10., 160., 40.));
        scene.set_paint_transform(node, Affine::scale(0., 1.));
        scene.flush();
        assert!(!scene.hit_test_all(0., 5.).contains(&node));
        scene.set_paint_transform(node, Affine::scale(f32::NAN, 1.));
        scene.flush();
        assert_eq!(scene.paint_transform(node), Affine::IDENTITY);
    }
    #[test]
    fn projection_preserves_user_scale_pivot_when_allocation_resizes() {
        let mut scene = Scene::new(300., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(100.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform_origin(node, Affine::scale(2., 1.), [0.5, 0.5]);
        scene.flush();
        assert_eq!(scene.bounds(node).x, -50.);
        let mut style = scene.style(node);
        style.width = Some(200.);
        scene.set_style(node, style);
        scene.prepare_layout();
        scene.set_projection_transform(node, Affine::scale(0.5, 1.));
        scene.flush();
        assert_eq!(scene.bounds(node).x, -50.);
        assert_eq!(scene.bounds(node).width, 200.);
    }
    #[test]
    fn projection_solver_roundtrips_rotation_translation_and_normalized_pivot() {
        let mut scene = Scene::new(300., 300.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(80.),
                height: Some(40.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform_origin(
            node,
            Affine::rotation(0.7).then(Affine::translation(3., -2.)),
            [0.25, 0.75],
        );
        scene.set_transform(node, Transform { x: 7., y: 5. });
        let expected = Affine::scale(0.6, 1.4).then(Affine::translation(8., -9.));
        scene.set_projection_transform(node, expected);
        let desired = scene.local_paint_transform(node);
        scene.set_projection_transform(node, Affine::IDENTITY);
        let actual = scene.projection_for_paint(node, desired);
        for (actual, expected) in [
            (actual.a, expected.a),
            (actual.b, expected.b),
            (actual.c, expected.c),
            (actual.d, expected.d),
            (actual.tx, expected.tx),
            (actual.ty, expected.ty),
        ] {
            assert!(
                (actual - expected).abs() < 0.0001,
                "actual={actual} expected={expected}"
            );
        }
    }
    #[test]
    fn nonuniform_projection_preserves_rotated_primitive_axes() {
        let mut scene = Scene::new(300., 300.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(100.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform_origin(
            node,
            Affine::rotation(std::f32::consts::FRAC_PI_2),
            [0.5, 0.5],
        );
        scene.flush();
        let before = scene.bounds(node);
        let mut style = scene.style(node);
        style.width = Some(200.);
        scene.set_style(node, style);
        scene.prepare_layout();
        scene.set_projection_transform(node, Affine::scale(0.5, 1.));
        scene.flush();
        let after = scene.bounds(node);
        for (before, after) in [
            (before.x, after.x),
            (before.y, after.y),
            (before.width, after.width),
            (before.height, after.height),
        ] {
            assert!(
                (before - after).abs() < 0.0001,
                "before={before} after={after}"
            );
        }
    }
    #[test]
    fn rotated_clip_rejects_aabb_corners_and_keeps_unclipped_axis_overflow() {
        let mut scene = Scene::new(300., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(20.),
                height: Some(20.),
                clip: true,
                ..Default::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(60.),
                flex_shrink: 0.,
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform(parent, Affine::rotation(std::f32::consts::FRAC_PI_4));
        scene.set_transform(parent, Transform { x: 100., y: 50. });
        scene.flush();
        assert!(!scene.hit_test_all(87., 51.).contains(&child));
        assert!(scene.hit_test_all(100., 64.).contains(&child));
        let mut style = scene.style(parent);
        style.layout_options = Some(Arc::new(crate::layout::LayoutOptions {
            clip_x: Some(true),
            clip_y: Some(false),
            ..Default::default()
        }));
        scene.set_style(parent, style);
        scene.flush();
        let (x, y) = scene.local_to_world(parent, 10., 40.);
        assert!(scene.hit_test_all(x, y).contains(&child));
        let item = scene
            .layer_items(None)
            .into_iter()
            .find(|item| item.id == child)
            .unwrap();
        assert!(item.clip.unwrap().height > scene.bounds(parent).height);
        assert_eq!(item.clip_regions.len(), 1);
    }
    #[test]
    fn isolated_sampling_halo_is_damaged_and_survives_wrapper_culling() {
        let mut scene = Scene::new(200., 200.);
        let wrapper = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(80.),
                height: Some(80.),
                ..Default::default()
            },
        );
        let node = scene.append(
            wrapper,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_isolated(node, true);
        scene.set_transform(wrapper, Transform { x: 70.8, y: 70.8 });
        scene.set_paint_transform(node, Affine::rotation(0.3));
        scene.flush();
        let bounds = scene.bounds(node);
        let halo = Rect::new(bounds.x - 0.75, bounds.y + bounds.height * 0.5, 0.25, 0.25);
        assert!(
            scene
                .layer_items_within(None, Some(&[halo]))
                .iter()
                .any(|item| item.id == node)
        );
        scene.set_paint_transform(node, Affine::rotation(0.4));
        let report = scene.flush();
        assert!(report.damage.iter().any(|damage| damage.intersects(halo)));
    }
    #[test]
    fn user_world_hierarchy_matches_full_world_when_projection_is_identity() {
        let mut scene = Scene::new(300., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(100.),
                height: Some(100.),
                ..Default::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_paint_transform_origin(parent, Affine::scale(2., 0.5), [0.5, 0.5]);
        scene.set_transform(parent, Transform { x: 70., y: 80. });
        scene.set_paint_transform_origin(
            child,
            Affine::rotation(0.7).then(Affine::translation(3., -2.)),
            [0.25, 0.75],
        );
        let full = scene.world_paint_transform(child);
        let user = scene.world_user_transform(child);
        for point in [(0., 0.), (10., 5.), (20., 20.)] {
            let actual = user.point(point.0, point.1);
            let expected = full.point(point.0, point.1);
            assert!(
                (actual.0 - expected.0).abs() < 0.0001 && (actual.1 - expected.1).abs() < 0.0001
            );
        }
    }
    #[test]
    fn transformed_fade_uses_local_y_and_isolated_flip_invalidates_cached_pixels() {
        let mut scene = Scene::new(200., 200.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(40.),
                height: Some(40.),
                clip: true,
                fade_edges: [8., 4.],
                ..Default::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(10.),
                height: Some(10.),
                ..Default::default()
            },
        );
        scene.set_isolated(child, true);
        scene.set_transform(parent, Transform { x: 100., y: 100. });
        scene.flush();
        let revision = scene.layer_revision(child);
        scene.set_paint_transform_origin(
            parent,
            Affine::rotation(std::f32::consts::FRAC_PI_2),
            [0.5, 0.5],
        );
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0);
        assert_eq!(report.paint_nodes, 0);
        assert!(scene.layer_revision(child) > revision);
        let item = scene.paint_items().find(|item| item.id == child).unwrap();
        let (x, y) = scene.local_to_world(parent, 10., 3.);
        let (_, local_y) = item.fade_transform.point(x, y);
        assert!((local_y - (scene.layout_bounds(parent).y + 3.)).abs() < 0.0001);
        assert_eq!(item.mask.unwrap().1, [8., 4.]);
    }
    #[test]
    fn affine_damage_covers_old_new_ink_and_scroll_copy_falls_back() {
        let mut scene = Scene::new(300., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(80.),
                height: Some(80.),
                clip: true,
                ..Default::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.set_transform(parent, Transform { x: 60., y: 60. });
        scene.flush();
        scene.set_paint_transform(parent, Affine::rotation(std::f32::consts::FRAC_PI_2));
        let report = scene.flush();
        for point in [(65., 65.), (45., 65.)] {
            assert!(
                report
                    .damage
                    .iter()
                    .any(|damage| contains_point(*damage, point.0, point.1))
            );
        }
        scene.set_scroll_copy(child, true);
        scene.set_transform(child, Transform { x: 0., y: -3. });
        let report = scene.flush();
        assert!(scene.scroll_moves().is_empty());
        assert!(!report.damage.is_empty());
    }
    #[test]
    fn damage_keeps_thin_strips_disjoint_without_inflating_the_viewport() {
        let mut regions = Vec::new();
        merge_damage(&mut regions, Rect::new(0., 0., 1000., 8.), None);
        merge_damage(&mut regions, Rect::new(992., 0., 8., 800.), None);
        assert_eq!(
            regions.iter().map(|r| r.width * r.height).sum::<f32>(),
            14336.
        );
        // Overlapping additions must terminate, cover every input pixel and
        // keep alpha pixels in exactly one repaint region.
        let mut regions = Vec::new();
        let mut covered = [false; 64 * 64];
        for i in 0..120 {
            let (x, y) = ((i * 17) % 48, (i * 29) % 48);
            let rect = Rect::new(x as f32, y as f32, 16., 16.);
            merge_damage(&mut regions, rect, Some(4.));
            for y in y..y + 16 {
                for x in x..x + 16 {
                    covered[y * 64 + x] = true;
                }
            }
            for (index, a) in regions.iter().enumerate() {
                assert!(regions[index + 1..].iter().all(|b| !a.intersects(*b)));
            }
            for (index, pixel) in covered.iter().enumerate() {
                if *pixel {
                    let (x, y) = ((index % 64) as f32 + 0.5, (index / 64) as f32 + 0.5);
                    assert!(
                        regions.iter().any(|r| r.x <= x
                            && x < r.x + r.width
                            && r.y <= y
                            && y < r.y + r.height)
                    );
                }
            }
        }
    }

    #[test]
    fn empty_mirrored_scroll_extent_and_nested_axes_preserve_rigid_moves() {
        use crate::{compose::prelude::*, widgets::Ui};
        let mut ui = Ui::new(160., 120.);
        let x = ui.signal(0.);
        let y = ui.signal(0.);
        ui.mount(
            overlay()
                .w_full()
                .h_full()
                .child(
                    scroll_x(x.clone()).w_full().h_full().child(
                        scroll(y.clone())
                            .w(300.)
                            .h_full()
                            .child(div().size(300., 360.).bg(rgb(0x223344))),
                    ),
                )
                .child(
                    scroll(y.clone())
                        .absolute()
                        .right(0.)
                        .w(8.)
                        .h_full()
                        .child(div().size(8., 360.)),
                ),
        );
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        y.set(12.);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        assert_eq!(ui.scene.borrow().scroll_moves().len(), 1);
        x.set(12.);
        y.set(24.);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let scene = ui.scene.borrow();
        assert_eq!(scene.scroll_moves().len(), 1);
        assert_eq!(
            (scene.scroll_moves()[0].dx, scene.scroll_moves()[0].dy),
            (-12., -12.)
        );
    }
    fn text(value: &str) -> NodeKind {
        NodeKind::Text {
            text: value.into(),
            color: Color(255, 255, 255, 255),
            font_size: 16.0,
        }
    }
    #[test]
    fn flex_distribution_respects_limits_and_resizes() {
        let mut scene = Scene::new(300.0, 80.0);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Row));
        let a = scene.append(
            scene.root(),
            text("a"),
            Style {
                width: Some(50.0),
                max_width: Some(70.0),
                flex_grow: 1.0,
                ..Style::default()
            },
        );
        let b = scene.append(
            scene.root(),
            text("b"),
            Style {
                width: Some(50.0),
                flex_grow: 1.0,
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(a).width, 70.0);
        assert_eq!(scene.bounds(b).width, 230.0);
        scene.resize(400.0, 80.0);
        let report = scene.flush();
        assert_eq!(scene.bounds(b).width, 330.0);
        assert!(!report.damage.is_empty());
        assert!(scene.flush().is_idle());
    }
    #[test]
    fn flex_shrink_and_margins_are_included_in_allocation() {
        let mut scene = Scene::new(140.0, 80.0);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Row));
        let a = scene.append(
            scene.root(),
            text("a"),
            Style {
                width: Some(100.0),
                min_width: Some(80.0),
                flex_shrink: 1.0,
                ..Style::default()
            },
        );
        let b = scene.append(
            scene.root(),
            text("b"),
            Style {
                width: Some(100.0),
                flex_shrink: 1.0,
                margin: Insets::all(5.0),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(a).width, 80.0);
        assert_eq!(scene.bounds(b), Rect::new(85.0, 5.0, 50.0, 23.0));
    }
    #[test]
    fn text_measurement_reflows_when_parent_constraint_changes() {
        let mut scene = Scene::new(100.0, 300.0);
        scene.set_text_measurer(|text: &str, _: f32, limit: Option<f32>| {
            let width = text.len() as f32 * 10.0;
            let limit = limit.unwrap_or(width).max(1.0);
            (width.min(limit), (width / limit).ceil() * 20.0)
        });
        assert_eq!(
            scene.measure_text("abcdefghijabcdefghij", 16.0, Some(50.0)),
            (50.0, 80.0)
        );
        let a = scene.append(
            scene.root(),
            text("abcdefghijabcdefghij"),
            Style {
                text_wrap: true,
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(a).height, 40.0);
        scene.resize(50.0, 300.0);
        scene.flush();
        assert_eq!(scene.bounds(a).height, 80.0);
    }
    #[test]
    fn text_sizes_survive_alternating_wrap_limits() {
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = calls.clone();
        let mut scene = Scene::new(300.0, 100.0);
        scene.set_text_measurer(move |text: &str, _: f32, _: Option<f32>| {
            counter.set(counter.get() + 1);
            (text.len() as f32 * 10.0, 20.0)
        });
        let label = scene.append(
            scene.root(),
            text("label"),
            Style {
                text_wrap: true,
                ..Style::default()
            },
        );
        let mut sizes = TextSizes::default();
        for limit in [Some(100.), Some(200.), Some(100.), Some(200.)] {
            if sizes.get(limit, 0).is_none() {
                sizes.insert(limit, 0, (1., 1.));
            }
        }
        assert_eq!(sizes.0.iter().flatten().count(), 2);
        scene.flush();
        let first = calls.get();
        // Relayout of the parent re-measures the clean label at both widths.
        for width in [200.0, 100.0, 200.0, 100.0] {
            scene.set_style(
                scene.root(),
                Style {
                    width: Some(width),
                    height: Some(100.0),
                    ..Style::default()
                },
            );
            scene.flush();
        }
        assert!(
            calls.get() <= first + 2,
            "{} measurements",
            calls.get() - first
        );
        let _ = label;
    }
    #[test]
    fn cached_text_size_follows_text_and_measurer_changes() {
        let mut scene = Scene::new(300.0, 100.0);
        scene.set_text_measurer(|text: &str, _: f32, _: Option<f32>| {
            (text.len() as f32 * 10.0, 20.0)
        });
        let label = scene.append(scene.root(), text("label"), Style::default());
        scene.flush();
        assert_eq!(scene.bounds(label).width, 50.0);
        scene.set_kind(label, text("longer label"));
        scene.flush();
        assert_eq!(scene.bounds(label).width, 120.0);
        scene.set_text_measurer(|text: &str, _: f32, _: Option<f32>| {
            (text.len() as f32 * 5.0, 20.0)
        });
        scene.flush();
        assert_eq!(scene.bounds(label).width, 60.0);
    }
    #[test]
    fn alignment_stretch_and_justification_apply_to_children() {
        let mut scene = Scene::new(100.0, 100.0);
        scene.set_style(
            scene.root(),
            Style {
                width: Some(100.0),
                height: Some(100.0),
                align: Align::Stretch,
                justify: Justify::SpaceBetween,
                ..Style::default()
            },
        );
        let a = scene.append(scene.root(), text("a"), Style::default());
        let b = scene.append(scene.root(), text("b"), Style::default());
        scene.flush();
        assert_eq!(scene.bounds(a), Rect::new(0.0, 0.0, 100.0, 23.0));
        assert_eq!(scene.bounds(b), Rect::new(0.0, 77.0, 100.0, 23.0));
    }
    #[test]
    fn hit_testing_respects_stacking_clip_transform_and_opacity() {
        let mut scene = Scene::new(100.0, 100.0);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let a = scene.append(scene.root(), text("aaaaaaaa"), Style::default());
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(20.0),
                height: Some(20.0),
                clip: true,
                ..Style::default()
            },
        );
        let child = scene.append(parent, text("bbbbbbbb"), Style::default());
        scene.set_transform(parent, Transform { x: 10.0, y: 0.0 });
        scene.flush();
        assert_eq!(scene.hit_test(15.0, 10.0), Some(child));
        assert_eq!(scene.hit_test(35.0, 10.0), Some(a));
        assert_eq!(
            scene.ancestors(child).collect::<Vec<_>>(),
            vec![parent, scene.root()]
        );
        scene.set_effects(
            parent,
            Effects {
                opacity: 0.0,
                ..Effects::default()
            },
        );
        assert_eq!(scene.hit_test(15.0, 10.0), Some(a));
        assert_eq!(scene.hit_test(110.0, 10.0), None);
    }
    #[test]
    fn preparing_input_geometry_preserves_damage_for_renderer() {
        let mut scene = Scene::new(200.0, 200.0);
        let node = scene.append(scene.root(), text("first"), Style::default());
        scene.prepare_layout();
        assert!(scene.bounds(node).width > 0.0);
        scene.set_text(node, "longer updated text");
        scene.prepare_layout();
        let report = scene.flush();
        assert!(report.layout_nodes >= 4);
        assert!(!report.damage.is_empty());
        assert!(scene.flush().is_idle());
    }
    #[test]
    fn idle_and_equal_writes_do_no_work() {
        let mut scene = Scene::new(800.0, 600.0);
        let id = scene.append(scene.root(), text("hi"), Style::default());
        assert!(!scene.flush().damage.is_empty());
        assert!(scene.flush().is_idle());
        scene.set_text(id, "hi");
        assert!(scene.flush().is_idle());
    }
    #[test]
    fn fixed_text_invalidates_only_its_paint() {
        let mut scene = Scene::new(800.0, 600.0);
        let a = scene.append(
            scene.root(),
            text("a"),
            Style {
                width: Some(200.0),
                height: Some(24.0),
                ..Style::default()
            },
        );
        scene.append(scene.root(), text("stable"), Style::default());
        scene.flush();
        scene.set_text(a, "stream");
        let frame = scene.flush();
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(frame.paint_nodes, 1);
        // Headless measurement is approximate: the whole box.
        assert_eq!(frame.damage, vec![Rect::new(0.0, 0.0, 200.0, 24.0)]);
    }
    #[test]
    fn transforms_damage_old_and_new_without_layout_or_paint() {
        let mut scene = Scene::new(800.0, 600.0);
        let id = scene.append(
            scene.root(),
            NodeKind::Rect(Color(1, 2, 3, 255)),
            Style {
                width: Some(20.0),
                height: Some(20.0),
                ..Style::default()
            },
        );
        scene.flush();
        scene.set_transform(id, Transform { x: 100.0, y: 0.0 });
        let frame = scene.flush();
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(frame.paint_nodes, 0);
        assert_eq!(frame.composite_nodes, 1);
        assert!(frame.damage.contains(&Rect::new(0.0, 0.0, 20.0, 20.0)));
        assert!(frame.damage.contains(&Rect::new(100.0, 0.0, 20.0, 20.0)));
    }
    #[test]
    fn reused_slots_do_not_resurrect_handles() {
        let mut scene = Scene::new(100.0, 100.0);
        let old = scene.append(scene.root(), text("old"), Style::default());
        scene.remove(old);
        let new = scene.append(scene.root(), text("new"), Style::default());
        assert!(!scene.contains(old));
        assert!(scene.contains(new));
        assert_ne!(old, new);
        scene.flush();
    }
    #[test]
    fn opacity_inherits_and_clips_follow_transforms() {
        let mut scene = Scene::new(100.0, 100.0);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(20.0),
                height: Some(20.0),
                clip: true,
                ..Style::default()
            },
        );
        let child = scene.append(parent, text("hello"), Style::default());
        scene.flush();
        scene.set_effects(
            parent,
            Effects {
                opacity: 0.5,
                ..Effects::default()
            },
        );
        scene.set_effects(
            child,
            Effects {
                opacity: 0.5,
                ..Effects::default()
            },
        );
        scene.set_transform(parent, Transform { x: 10.0, y: 10.0 });
        scene.flush();
        let item = scene.paint_items().find(|i| i.id == child).unwrap();
        assert_eq!(item.effects.opacity, 0.25);
        assert_eq!(item.clip, Some(Rect::new(10.0, 10.0, 20.0, 20.0)));
    }
    #[test]
    fn backdrop_damage_expands_to_filter_output() {
        let mut scene = Scene::new(300.0, 300.0);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let a = scene.append(
            scene.root(),
            text("a"),
            Style {
                width: Some(20.0),
                height: Some(20.0),
                ..Style::default()
            },
        );
        let b = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 255, 255, 128)),
            Style {
                width: Some(100.0),
                height: Some(100.0),
                ..Style::default()
            },
        );
        scene.set_effects(
            b,
            Effects {
                blur_radius: 4.0,
                ..Effects::default()
            },
        );
        scene.flush();
        scene.set_text(a, "b");
        let frame = scene.flush();
        assert!(
            frame
                .damage
                .iter()
                .any(|r| r.width >= 100.0 && r.height >= 100.0)
        );
    }
    #[test]
    fn natural_text_layout_preserves_unaffected_subtrees() {
        let mut scene = Scene::new(500.0, 500.0);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let changed = scene.append(scene.root(), text("a"), Style::default());
        let stable = scene.append(scene.root(), text("stable"), Style::default());
        scene.set_transform(stable, Transform { x: 200.0, y: 200.0 });
        scene.flush();
        scene.set_text(changed, "longer");
        let frame = scene.flush();
        assert_eq!(frame.layout_nodes, 2); // Root and the changed leaf; stable is cached.
        assert_eq!(frame.paint_nodes, 1);
        assert!(
            frame
                .damage
                .iter()
                .all(|rect| !rect.intersects(scene.bounds(stable)))
        );
    }
    #[test]
    fn intrinsic_height_change_moves_and_damages_following_sibling() {
        let mut scene = Scene::new(500.0, 500.0);
        let first = scene.append(scene.root(), text("a"), Style::default());
        let second = scene.append(scene.root(), text("b"), Style::default());
        scene.flush();
        let old = scene.bounds(second);
        scene.set_text(first, "a\na");
        let frame = scene.flush();
        let new = scene.bounds(second);
        assert_eq!(old.y, 23.0);
        assert_eq!(new.y, 46.0);
        assert!(frame.damage.iter().any(|rect| rect.intersects(old)));
        assert!(frame.damage.iter().any(|rect| rect.intersects(new)));
        assert!(scene.flush().is_idle());
    }
    #[test]
    fn removing_a_subtree_clears_its_pixels_and_reclaims_every_node() {
        let mut scene = Scene::new(500.0, 500.0);
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style::default(),
        );
        let child = scene.append(group, text("removed"), Style::default());
        scene.flush();
        let old = scene.bounds(child);
        scene.remove(group);
        let frame = scene.flush();
        assert_eq!(scene.len(), 1);
        assert!(!scene.contains(child));
        assert!(frame.damage.iter().any(|rect| rect.intersects(old)));
        assert!(scene.flush().is_idle());
    }
    #[test]
    fn parent_transform_damages_descendant_at_both_locations() {
        let mut scene = Scene::new(500.0, 500.0);
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style::default(),
        );
        let child = scene.append(group, text("moving"), Style::default());
        scene.flush();
        let old = scene.bounds(child);
        scene.set_transform(group, Transform { x: 200.0, y: 200.0 });
        let new = scene.bounds(child);
        let frame = scene.flush();
        assert_eq!(frame.layout_nodes, 0);
        assert_eq!(frame.paint_nodes, 0);
        assert!(frame.damage.iter().any(|rect| rect.intersects(old)));
        assert!(frame.damage.iter().any(|rect| rect.intersects(new)));
    }
    #[test]
    fn expanding_clip_reveals_and_damages_previously_hidden_content() {
        let mut scene = Scene::new(500.0, 500.0);
        let mut style = Style {
            width: Some(20.0),
            height: Some(20.0),
            clip: true,
            ..Style::default()
        };
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            style.clone(),
        );
        scene.append(
            group,
            NodeKind::Rect(Color(255, 255, 255, 255)),
            Style {
                width: Some(100.0),
                height: Some(20.0),
                ..Style::default()
            },
        );
        scene.flush();
        style.clip = false;
        scene.set_style(group, style);
        let frame = scene.flush();
        assert!(frame.damage.iter().any(|rect| rect.x + rect.width >= 100.0));
    }
    #[test]
    fn resizing_clip_damages_newly_visible_pixels() {
        let mut scene = Scene::new(500.0, 500.0);
        let mut style = Style {
            width: Some(20.0),
            height: Some(20.0),
            clip: true,
            ..Style::default()
        };
        let group = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            style.clone(),
        );
        scene.append(
            group,
            NodeKind::Rect(Color(255, 255, 255, 255)),
            Style {
                width: Some(100.0),
                height: Some(20.0),
                ..Style::default()
            },
        );
        scene.flush();
        style.width = Some(100.0);
        scene.set_style(group, style);
        let frame = scene.flush();
        assert!(frame.damage.iter().any(|rect| rect.x + rect.width >= 100.0));
    }
    #[test]
    fn fractional_damage_is_rounded_to_cover_complete_pixels() {
        let mut scene = Scene::new(100.0, 100.0);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(100, 100, 100, 100)),
            Style {
                width: Some(10.0),
                height: Some(10.0),
                ..Style::default()
            },
        );
        scene.flush();
        scene.set_transform(node, Transform { x: 20.25, y: 20.75 });
        let frame = scene.flush();
        assert!(frame.damage.contains(&Rect::new(20.0, 20.0, 11.0, 11.0)));
        assert!(frame.damage.iter().all(|r| {
            [r.x, r.y, r.width, r.height]
                .into_iter()
                .all(|v| v.fract() == 0.0)
        }));
    }
}

#[cfg(test)]
mod paint_geometry_tests {
    use super::*;
    #[test]
    fn overlay_alignment_centers_each_layer_independently_in_both_layout_paths() {
        for advanced in [false, true] {
            let mut scene = Scene::new(100., 100.);
            let parent = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Overlay),
                Style {
                    width: Some(80.),
                    height: Some(60.),
                    padding_edges: Some(Insets {
                        left: 4.,
                        right: 8.,
                        top: 2.,
                        bottom: 6.,
                    }),
                    align: Align::Center,
                    justify: Justify::Center,
                    layout_options: advanced.then(|| {
                        Arc::new(crate::layout::LayoutOptions {
                            gap_x: Some(crate::layout::Length::Px(0.)),
                            ..Default::default()
                        })
                    }),
                    ..Default::default()
                },
            );
            for (w, h) in [(20., 10.), (12., 18.)] {
                scene.append(
                    parent,
                    NodeKind::Rect(Color(255, 255, 255, 255)),
                    Style {
                        width: Some(w),
                        height: Some(h),
                        ..Default::default()
                    },
                );
            }
            let anchored = scene.append(
                parent,
                NodeKind::Rect(Color(255, 0, 0, 255)),
                Style {
                    absolute: true,
                    width: Some(5.),
                    height: Some(5.),
                    margin: Insets {
                        left: 3.,
                        top: 4.,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            scene.flush();
            for child in scene
                .children(parent)
                .iter()
                .copied()
                .filter(|child| *child != anchored)
            {
                let rect = scene.bounds(child);
                assert_eq!(
                    (rect.x + rect.width / 2., rect.y + rect.height / 2.),
                    (38., 28.),
                    "advanced={advanced}"
                );
            }
            assert_eq!(
                (scene.bounds(anchored).x, scene.bounds(anchored).y),
                (7., 6.)
            );
        }
    }
    #[test]
    fn rect_and_quad_paint_changes_preserve_layout() {
        let mut scene = Scene::new(200., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(100.),
                height: Some(40.),
                ..Style::default()
            },
        );
        scene.flush();
        let bounds = scene.bounds(node);
        for kind in [
            NodeKind::Rect(Color(0, 255, 0, 255)),
            NodeKind::Quad(QuadStyle {
                fill: Color(0, 0, 255, 255),
                radius: 8.,
                ..QuadStyle::default()
            }),
        ] {
            scene.set_kind(node, kind);
            let report = scene.flush();
            assert_eq!(report.layout_nodes, 0);
            assert!(!report.damage.is_empty());
            assert_eq!(scene.bounds(node), bounds);
        }
        assert!(scene.flush().is_idle());
    }
}

#[cfg(test)]
mod deferred_frame_tests {
    use super::*;
    #[test]
    fn unflushed_node_replacement_bounds_stale_dirty_generations() {
        let mut scene = Scene::new(100., 100.);
        scene.flush();
        for i in 0..100_000 {
            let node = scene.append(
                scene.root(),
                NodeKind::Rect(Color((i % 255) as u8, 0, 0, 255)),
                Style {
                    width: Some(10.),
                    height: Some(10.),
                    ..Style::default()
                },
            );
            scene.remove(node);
            assert!(scene.dirty.len() <= 64);
        }
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(0, 255, 0, 255)),
            Style {
                width: Some(10.),
                height: Some(10.),
                ..Style::default()
            },
        );
        let report = scene.flush();
        assert!(!report.damage.is_empty());
        assert_eq!(scene.bounds(node).width, 10.);
        assert!(scene.flush().is_idle());
        scene.set_transform(node, Transform { x: 20., y: 0. });
        assert!(!scene.flush().damage.is_empty());
    }
}

#[cfg(test)]
mod absolute_layout_tests {
    use super::*;
    #[test]
    fn absolute_children_do_not_change_flow_intrinsic_size_or_gaps() {
        for layout in [Layout::Row, Layout::Column, Layout::Overlay] {
            let mut scene = Scene::new(500., 500.);
            let parent = scene.append(
                scene.root(),
                NodeKind::Container(layout),
                Style {
                    padding: 5.,
                    gap: 7.,
                    ..Style::default()
                },
            );
            let first = scene.append(
                parent,
                NodeKind::Rect(Color(255, 0, 0, 255)),
                Style {
                    width: Some(20.),
                    height: Some(30.),
                    ..Style::default()
                },
            );
            let floating = scene.append(
                parent,
                NodeKind::Rect(Color(0, 255, 0, 255)),
                Style {
                    absolute: true,
                    width: Some(200.),
                    height: Some(300.),
                    margin: Insets {
                        left: 3.,
                        top: 4.,
                        ..Insets::default()
                    },
                    flex_grow: 100.,
                    ..Style::default()
                },
            );
            let second = scene.append(
                parent,
                NodeKind::Rect(Color(0, 0, 255, 255)),
                Style {
                    width: Some(20.),
                    height: Some(30.),
                    ..Style::default()
                },
            );
            scene.flush();
            let expected = match layout {
                Layout::Row => (57., 40.),
                Layout::Column => (30., 77.),
                Layout::Overlay => (30., 40.),
            };
            assert_eq!(
                (scene.bounds(parent).width, scene.bounds(parent).height),
                expected
            );
            assert_eq!(scene.bounds(first), Rect::new(5., 5., 20., 30.));
            assert_eq!(scene.bounds(floating), Rect::new(8., 9., 200., 300.));
            let before = scene.bounds(second);
            scene.remove(floating);
            scene.flush();
            assert_eq!(scene.bounds(second), before);
            assert_eq!(
                (scene.bounds(parent).width, scene.bounds(parent).height),
                expected
            );
        }
    }
    #[test]
    fn absolute_text_remeasures_on_parent_resize_without_taking_flex_space() {
        let mut scene = Scene::new(200., 100.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Row));
        let sibling = scene.append(
            scene.root(),
            NodeKind::Rect(Color(0, 0, 0, 255)),
            Style {
                width: Some(20.),
                height: Some(10.),
                flex_grow: 1.,
                ..Style::default()
            },
        );
        let text = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "one two three four five six seven eight nine ten".into(),
                color: Color(255, 255, 255, 255),
                font_size: 16.,
            },
            Style {
                absolute: true,
                text_wrap: true,
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(sibling).width, 200.);
        let height = scene.bounds(text).height;
        scene.resize(80., 100.);
        scene.flush();
        assert_eq!(scene.bounds(sibling).width, 80.);
        assert!(scene.bounds(text).height > height);
    }
    #[test]
    fn geometry_revision_tracks_layout_and_transforms_but_not_equal_transforms() {
        let mut scene = Scene::new(100., 100.);
        let before = scene.geometry_revision();
        scene.prepare_layout();
        assert_ne!(scene.geometry_revision(), before);
        let before = scene.geometry_revision();
        scene.prepare_layout();
        assert_eq!(scene.geometry_revision(), before);
        scene.set_transform(scene.root(), Transform { x: 1., y: 2. });
        assert_ne!(scene.geometry_revision(), before);
        let before = scene.geometry_revision();
        scene.set_transform(scene.root(), Transform { x: 1., y: 2. });
        assert_eq!(scene.geometry_revision(), before);
    }
}

#[cfg(test)]
mod line_height_tests {
    use super::*;
    use crate::text_layout::{FallbackTextLayout, FontStyle, LineHeight, TextLayout};

    #[test]
    fn explicit_pitch_relayouts_once_and_native_font_callback_receives_metrics() {
        let mut scene = Scene::new(200., 200.);
        scene.set_font_text_shaper(
            |text: &str, size, width, font: &FontStyle| -> Box<dyn TextLayout> {
                Box::new(FallbackTextLayout::with_line_height(
                    text,
                    size,
                    width,
                    font.line_height,
                ))
            },
        );
        let node = scene.append(
            scene.root(),
            NodeKind::Text {
                text: "one\ntwo\n".into(),
                font_size: 10.,
                color: Color(255, 255, 255, 255),
            },
            Style::default(),
        );
        scene.flush();
        assert_eq!(scene.bounds(node).height, 42.);
        let font = FontStyle {
            line_height: LineHeight::px(24.),
            ..FontStyle::default()
        };
        scene.set_font(node, font.clone());
        let changed = scene.flush();
        assert!(changed.layout_nodes > 0 && !changed.damage.is_empty());
        assert_eq!(scene.bounds(node).height, 72.);
        assert_eq!(
            scene
                .shape_text_with_font("a\nb", 10., None, &font)
                .caret(2)
                .y,
            24.
        );
        scene.set_font(node, font);
        assert!(scene.flush().is_idle());
    }

    #[test]
    fn fallback_measurement_matches_grapheme_shaping_for_normal_and_explicit_pitch() {
        let scene = Scene::new(100., 100.);
        for line_height in [LineHeight::NORMAL, LineHeight::px(3.), LineHeight::px(30.)] {
            let font = FontStyle {
                line_height,
                ..FontStyle::default()
            };
            let text = "e\u{301}👩‍💻\r\nx";
            let measured = scene.measure_text_with_font(text, 10., Some(6.), &font);
            let shaped = scene.shape_text_with_font(text, 10., Some(6.), &font);
            assert_eq!(measured, shaped.size());
            assert_eq!(measured.1, 3. * line_height.resolve(10.));
        }
    }
}

#[cfg(test)]
mod percentage_layout_tests {
    use super::*;

    fn rect() -> NodeKind {
        NodeKind::Rect(Color(255, 0, 0, 255))
    }

    #[test]
    fn percentages_follow_definite_padded_parent_and_resize_without_idle_work() {
        let mut scene = Scene::new(400., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width_percent: Some(0.5),
                height_percent: Some(1.),
                padding: 10.,
                align: Align::Stretch,
                ..Style::default()
            },
        );
        let child = scene.append(
            parent,
            rect(),
            Style {
                width_percent: Some(0.5),
                height_percent: Some(0.5),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(child), Rect::new(10., 10., 90., 140.));
        scene.resize(600., 400.);
        scene.flush();
        assert_eq!(scene.bounds(child), Rect::new(10., 10., 140., 190.));
        assert!(scene.flush().is_idle());
        let unchanged = scene.style(child);
        scene.set_style(child, unchanged);
        assert!(scene.flush().is_idle());
    }

    #[test]
    fn percentages_supply_flex_bases_and_forced_allocation_supplies_descendant_basis() {
        let mut scene = Scene::new(400., 200.);
        let row = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Row),
            Style {
                width: Some(300.),
                height: Some(100.),
                ..Style::default()
            },
        );
        let first = scene.append(
            row,
            NodeKind::Container(Layout::Overlay),
            Style {
                width_percent: Some(0.75),
                height_percent: Some(1.),
                flex_shrink: 1.,
                ..Style::default()
            },
        );
        let second = scene.append(
            row,
            rect(),
            Style {
                width_percent: Some(0.75),
                height_percent: Some(1.),
                flex_shrink: 1.,
                ..Style::default()
            },
        );
        let nested = scene.append(
            first,
            rect(),
            Style {
                width_percent: Some(0.5),
                height_percent: Some(1.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(first).width, 150.);
        assert_eq!(scene.bounds(second).width, 150.);
        assert_eq!(scene.bounds(nested).width, 75.);
        assert_eq!(scene.bounds(nested).height, 100.);
    }

    #[test]
    fn indefinite_percentages_use_intrinsic_size_and_absolute_uses_final_content_box() {
        let mut scene = Scene::new(400., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                padding: 10.,
                ..Style::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Container(Layout::Overlay),
            Style {
                width_percent: Some(0.5),
                height_percent: Some(1.),
                ..Style::default()
            },
        );
        scene.append(
            child,
            rect(),
            Style {
                width: Some(80.),
                height: Some(40.),
                ..Style::default()
            },
        );
        let absolute = scene.append(
            parent,
            rect(),
            Style {
                absolute: true,
                width_percent: Some(0.5),
                height_percent: Some(0.5),
                margin: Insets::all(3.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(parent).width, 100.);
        assert_eq!(scene.bounds(parent).height, 60.);
        assert_eq!(scene.bounds(child).width, 80.);
        assert_eq!(scene.bounds(child).height, 40.);
        assert_eq!(scene.bounds(absolute), Rect::new(13., 13., 40., 20.));
        assert!(scene.flush().is_idle());
    }

    #[test]
    fn indefinite_percentage_uses_auto_cross_axis_stretch() {
        let mut scene = Scene::new(400., 300.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                align: Align::Stretch,
                ..Style::default()
            },
        );
        scene.append(
            parent,
            rect(),
            Style {
                width: Some(200.),
                height: Some(20.),
                ..Style::default()
            },
        );
        let child = scene.append(
            parent,
            NodeKind::Container(Layout::Overlay),
            Style {
                width_percent: Some(0.5),
                ..Style::default()
            },
        );
        let nested = scene.append(
            child,
            rect(),
            Style {
                width: Some(80.),
                height: Some(20.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(parent).width, 200.);
        assert_eq!(scene.bounds(child).width, 200.);
        assert_eq!(scene.bounds(nested).width, 80.);
        assert!(scene.flush().is_idle());
    }

    #[test]
    fn growing_row_provides_definite_height_for_percentage_container() {
        let mut scene = Scene::new(640., 420.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Style {
                width_percent: Some(1.),
                height_percent: Some(1.),
                padding: 20.,
                gap: 12.,
                ..Style::default()
            },
        );
        scene.append(
            parent,
            rect(),
            Style {
                width: Some(120.),
                height: Some(24.),
                ..Style::default()
            },
        );
        let row = scene.append(
            parent,
            NodeKind::Container(Layout::Row),
            Style {
                width_percent: Some(1.),
                flex_grow: 1.,
                ..Style::default()
            },
        );
        let right = scene.append(
            row,
            NodeKind::Container(Layout::Column),
            Style {
                width_percent: Some(0.5),
                height_percent: Some(1.),
                padding: 12.,
                ..Style::default()
            },
        );
        scene.append(
            right,
            rect(),
            Style {
                width: Some(80.),
                height: Some(74.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(row).height, 344.);
        assert_eq!(scene.bounds(right).height, 344.);
        for _ in 0..3 {
            scene.invalidate_layout(row);
            scene.prepare_layout();
            assert_eq!(scene.bounds(right).height, 344.);
        }
    }

    #[test]
    fn zero_delta_flex_allocation_is_definite_for_percentage_descendants() {
        let mut scene = Scene::new(200., 100.);
        let row = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Row),
            Style {
                flex_grow: 1.,
                ..Style::default()
            },
        );
        scene.append(
            row,
            rect(),
            Style {
                width: Some(50.),
                height: Some(100.),
                ..Style::default()
            },
        );
        let percent = scene.append(
            row,
            rect(),
            Style {
                width: Some(50.),
                height_percent: Some(1.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(row).height, 100.);
        assert_eq!(scene.bounds(percent).height, 100.);
        assert!(scene.flush().is_idle());
    }

    #[test]
    fn percentage_constraints_precedence_and_invalid_values_are_finite() {
        let mut scene = Scene::new(200., 100.);
        let child = scene.append(
            scene.root(),
            rect(),
            Style {
                width_percent: Some(2.),
                height_percent: Some(f32::INFINITY),
                max_width: Some(120.),
                min_height: Some(5.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(child).width, 120.);
        assert_eq!(scene.bounds(child).height, 5.);
        scene.set_style(
            child,
            Style {
                width: Some(37.),
                width_percent: Some(0.5),
                height_percent: Some(-1.),
                ..Style::default()
            },
        );
        scene.flush();
        assert_eq!(scene.bounds(child).width, 37.);
        assert_eq!(scene.bounds(child).height, 0.);
    }
}

#[cfg(test)]
mod letter_spacing_fallback_tests {
    use super::*;
    use crate::text_layout::{FontStyle, LetterSpacing, TextLayout};
    #[test]
    fn explicit_spacing_bypasses_legacy_callbacks_without_losing_metrics() {
        let mut scene = Scene::new(200., 100.);
        scene.set_text_shaper(|_: &str, _, _| -> Box<dyn TextLayout> {
            panic!("legacy callback cannot receive nonzero letter spacing")
        });
        let font = FontStyle {
            letter_spacing: LetterSpacing::px(4.),
            ..FontStyle::default()
        };
        let measured = scene.measure_text_with_font("abc", 10., Some(20.), &font);
        let shaped = scene.shape_text_with_font("abc", 10., Some(20.), &font);
        assert_eq!(measured, (20., 28.));
        assert_eq!(measured, shaped.size());
    }
}

#[cfg(test)]
mod image_replacement_layout_tests {
    use super::*;
    use crate::image::ImageData;
    use std::sync::Arc;
    fn pixels(w: u32, h: u32) -> NodeKind {
        NodeKind::Image(Arc::new(
            ImageData::new(w, h, [0, 0, 0, 255].repeat((w * h) as usize)).unwrap(),
        ))
    }
    #[test]
    fn fixed_axes_ignore_source_size_even_zero_and_constrained_but_auto_axes_remeasure() {
        for style in [
            Style {
                width: Some(0.),
                height: Some(0.),
                ..Default::default()
            },
            Style {
                width: Some(80.),
                height: Some(70.),
                max_width: Some(30.),
                min_height: Some(100.),
                width_percent: Some(0.5),
                ..Default::default()
            },
        ] {
            let mut scene = Scene::new(200., 200.);
            let image = scene.append(scene.root(), pixels(20, 10), style.clone());
            scene.flush();
            let before = scene.bounds(image);
            scene.set_kind(image, pixels(10, 20));
            let report = scene.flush();
            assert_eq!(report.layout_nodes, 0);
            assert_eq!(scene.bounds(image), before);
        }
        for style in [
            Style::default(),
            Style {
                width: Some(40.),
                ..Default::default()
            },
            Style {
                width_percent: Some(0.5),
                height_percent: Some(0.5),
                ..Default::default()
            },
        ] {
            let mut scene = Scene::new(200., 200.);
            let image = scene.append(scene.root(), pixels(20, 10), style.clone());
            scene.flush();
            scene.set_kind(image, pixels(10, 20));
            assert!(
                scene.flush().layout_nodes > 0,
                "non-concrete axes use existing invalidation"
            );
            if style.height.is_none() && style.height_percent.is_none() {
                assert_eq!(scene.bounds(image).height, 20.);
            }
        }
    }
}

#[cfg(test)]
mod hidden_layer_tests {
    use super::*;
    #[test]
    fn hidden_huge_descendants_do_not_expand_visible_layer_allocation() {
        let mut scene = Scene::new(300., 200.);
        let root = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Overlay),
            Style {
                width: Some(80.),
                height: Some(60.),
                ..Default::default()
            },
        );
        scene.set_isolated(root, true);
        let hidden = scene.append(
            root,
            NodeKind::Container(Layout::Overlay),
            Style {
                absolute: true,
                width: Some(1.),
                height: Some(1.),
                ..Default::default()
            },
        );
        let huge = scene.append(
            hidden,
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(10000.),
                height: Some(12000.),
                ..Default::default()
            },
        );
        scene.set_effects(
            hidden,
            Effects {
                opacity: 0.,
                ..Default::default()
            },
        );
        scene.flush();
        assert_eq!(scene.layer_bounds(root), scene.bounds(root));
        assert_eq!(
            scene
                .layer_items(Some(root))
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![root]
        );
        assert!(
            scene.paint_items().any(|p| p.id == huge),
            "public traversal retains cache liveness"
        );
        scene.set_effects(hidden, Effects::default());
        scene.flush();
        assert_eq!(scene.layer_bounds(root).width, 10000.);
        assert_eq!(scene.layer_bounds(root).height, 12000.);
    }

    #[test]
    fn hidden_nested_layers_prune_but_explicit_layer_root_opacity_stays_external() {
        let mut scene = Scene::new(300., 200.);
        let root = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(80.),
                height: Some(60.),
                ..Default::default()
            },
        );
        scene.set_isolated(root, true);
        let nested = scene.append(
            root,
            NodeKind::Rect(Color(0, 255, 0, 255)),
            Style {
                absolute: true,
                width: Some(10000.),
                height: Some(12000.),
                ..Default::default()
            },
        );
        scene.set_isolated(nested, true);
        scene.set_effects(
            nested,
            Effects {
                opacity: 0.,
                ..Default::default()
            },
        );
        scene.flush();
        assert_eq!(scene.layer_bounds(root), scene.bounds(root));
        assert!(!scene.layer_items(Some(root)).iter().any(|p| p.id == nested));
        assert!(scene.isolated_nodes().any(|id| id == nested));
        scene.set_effects(
            root,
            Effects {
                opacity: 0.,
                ..Default::default()
            },
        );
        scene.flush();
        assert!(!scene.layer_items(None).iter().any(|p| p.id == root));
        let cached = scene.layer_items(Some(root));
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].effects.opacity, 1.);
        assert_eq!(scene.layer_bounds(root), scene.bounds(root));
        assert_eq!(scene.layer_items(Some(nested))[0].effects.opacity, 1.);
    }
}

#[cfg(test)]
mod nonfinite_effect_tests {
    use super::*;
    #[test]
    fn invalid_effects_normalize_before_equality_and_paint_traversal() {
        let mut scene = Scene::new(100., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(50.),
                height: Some(50.),
                ..Default::default()
            },
        );
        scene.flush();
        let invalid = Effects {
            opacity: f32::NAN,
            blur_radius: f32::INFINITY,
            edge_fade: f32::NEG_INFINITY,
        };
        scene.set_effects(node, invalid);
        assert_eq!(scene.effects(node), Effects::default());
        assert!(scene.flush().is_idle());
        for _ in 0..3 {
            scene.set_effects(node, invalid);
            assert!(scene.flush().is_idle());
        }
        assert!(scene.paint_items().all(|p| p.effects.opacity.is_finite()
            && p.effects.blur_radius.is_finite()
            && p.effects.edge_fade.is_finite()));
        assert!(scene.blur_nodes.is_empty());
    }
    #[test]
    fn opacity_infinities_keep_clamped_endpoints_and_invalid_filters_remove_blur_tracking() {
        let mut scene = Scene::new(100., 100.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(50.),
                height: Some(50.),
                ..Default::default()
            },
        );
        scene.set_effects(
            node,
            Effects {
                opacity: 0.5,
                blur_radius: 4.,
                edge_fade: 3.,
            },
        );
        scene.flush();
        assert_eq!(scene.blur_nodes, vec![node]);
        scene.set_effects(
            node,
            Effects {
                opacity: f32::NEG_INFINITY,
                blur_radius: f32::NAN,
                edge_fade: f32::INFINITY,
            },
        );
        assert_eq!(
            scene.effects(node),
            Effects {
                opacity: 0.,
                ..Default::default()
            }
        );
        assert!(scene.blur_nodes.is_empty());
        assert!(!scene.layer_items(None).iter().any(|p| p.id == node));
        scene.flush();
        scene.set_effects(
            node,
            Effects {
                opacity: f32::INFINITY,
                blur_radius: -2.,
                edge_fade: -0.,
            },
        );
        assert_eq!(scene.effects(node), Effects::default());
        assert_eq!(scene.effects(node).edge_fade.to_bits(), 0);
        assert!(!scene.flush().damage.is_empty());
        scene.set_effects(
            node,
            Effects {
                opacity: 2.,
                blur_radius: f32::NEG_INFINITY,
                edge_fade: f32::NAN,
            },
        );
        assert!(scene.flush().is_idle());
    }
}

#[cfg(test)]
mod nonfinite_transform_tests {
    use super::*;
    #[test]
    fn invalid_axes_normalize_independently_before_equality_and_damage() {
        let mut scene = Scene::new(200., 150.);
        let node = scene.append(
            scene.root(),
            NodeKind::Rect(Color(255, 0, 0, 255)),
            Style {
                width: Some(30.),
                height: Some(20.),
                ..Default::default()
            },
        );
        scene.flush();
        scene.set_transform(node, Transform { x: 40., y: 25. });
        scene.flush();
        assert_eq!(scene.hit_test(45., 30.), Some(node));
        scene.set_transform(
            node,
            Transform {
                x: f32::NAN,
                y: 25.,
            },
        );
        assert_eq!(scene.transform(node), Transform { x: 0., y: 25. });
        let report = scene.flush();
        assert_eq!(report.layout_nodes, 0);
        assert!(!report.damage.is_empty());
        assert!(report.damage.iter().all(|r| r.x.is_finite()
            && r.y.is_finite()
            && r.width.is_finite()
            && r.height.is_finite()));
        assert_eq!(scene.hit_test(5., 30.), Some(node));
        assert_ne!(scene.hit_test(45., 30.), Some(node));
        let revision = scene.geometry_revision;
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0., 0.] {
            scene.set_transform(node, Transform { x, y: 25. });
            assert_eq!(scene.transform(node).x.to_bits(), 0);
            assert!(scene.flush().is_idle());
            assert_eq!(scene.geometry_revision, revision);
        }
        scene.set_transform(
            node,
            Transform {
                x: 12.,
                y: f32::INFINITY,
            },
        );
        assert_eq!(scene.transform(node), Transform { x: 12., y: 0. });
        assert_eq!(scene.hit_test(15., 5.), Some(node));
        assert!(!scene.flush().damage.is_empty());
        scene.set_transform(node, Transform { x: 40., y: 25. });
        assert_eq!(scene.hit_test(45., 30.), Some(node));
        assert!(!scene.flush().damage.is_empty());
    }
}

#[path = "scene_layout.rs"]
mod advanced_layout;

fn own_visible(style: &Style) -> bool {
    style.layout_options.as_deref().is_none_or(|options| {
        options.visible != Some(false) && options.display != Some(crate::layout::Display::None)
    })
}

#[cfg(test)]
mod cursor_count_tests {
    use super::*;
    #[test]
    fn clearing_and_removing_last_cursor_restores_default_fast_path() {
        let mut scene = Scene::new(100., 100.);
        let parent = scene.append(
            scene.root(),
            NodeKind::Container(Layout::Column),
            Default::default(),
        );
        let child = scene.append(
            parent,
            NodeKind::Container(Layout::Column),
            Default::default(),
        );
        scene.set_cursor(parent, Some(crate::cursor::Cursor::Grab));
        scene.set_cursor(child, Some(crate::cursor::Cursor::Grabbing));
        assert_eq!(scene.cursor_styles, 2);
        scene.set_cursor(child, Some(crate::cursor::Cursor::Pointer));
        assert_eq!(scene.cursor_styles, 2);
        scene.set_cursor(child, None);
        assert_eq!(scene.cursor_styles, 1);
        scene.remove(parent);
        assert_eq!(scene.cursor_styles, 0);
        assert_eq!(scene.cursor_at(10., 10.), None);
    }
}
