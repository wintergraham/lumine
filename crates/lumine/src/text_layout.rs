//! Platform-independent shaped text geometry consumed by editors and selection widgets.
use crate::scene::Rect;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
/// Portable font selection. Named families fall back through the platform font database.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
    Named(std::sync::Arc<str>),
}
impl From<&str> for FontFamily {
    fn from(value: &str) -> Self {
        Self::Named(value.into())
    }
}
impl From<String> for FontFamily {
    fn from(value: String) -> Self {
        Self::Named(value.into())
    }
}
/// Logical line-box pitch. Normal uses `ceil(font_size * 1.4)`.
///
/// Positive finite pixel values are preserved, including pitches smaller than
/// glyph ink. Text still obeys its allocated paint bounds: tight lines may
/// overlap internally and ink at the outer text-box edges may be clipped.
/// Private normalized bits give font caches stable equality and hashing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineHeight(u32, u32);
impl LineHeight {
    pub const NORMAL: Self = Self(0, 0);

    /// Invalid, zero, or negative values reset the normal line pitch.
    pub fn px(value: f32) -> Self {
        if value.is_finite() && value > 0. {
            Self(value.to_bits(), value.to_bits())
        } else {
            Self::NORMAL
        }
    }

    /// Round the line advance to logical pixels while retaining the natural
    /// line height for glyph baseline placement. Explicit `px` stays exact.
    pub fn rounded_px(value: f32) -> Self {
        if value.is_finite() && value > 0. {
            Self(value.round().max(1.).to_bits(), value.to_bits())
        } else {
            Self::NORMAL
        }
    }

    /// Baseline correction from rounded advance to natural line-box height.
    pub fn baseline_offset(self) -> f32 {
        (f32::from_bits(self.1) - f32::from_bits(self.0)) * 0.5
    }

    pub fn pixels(self) -> Option<f32> {
        (self != Self::NORMAL).then(|| f32::from_bits(self.0))
    }

    pub fn resolve(self, font_size: f32) -> f32 {
        self.pixels().unwrap_or_else(|| {
            let size = if font_size.is_finite() && font_size > 0. {
                font_size
            } else {
                1.
            };
            (size * 1.4).min(f32::MAX).ceil()
        })
    }
}
/// Additional tracking in logical pixels. Native glyph clusters follow the font
/// shaper; the fallback uses grapheme cells. Nonfinite values and signed zero
/// normalize to zero; finite values saturate at ±1,000,000 logical pixels to
/// keep native geometry practical and typography cache keys consistent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LetterSpacing(u32);
impl LetterSpacing {
    pub const ZERO: Self = Self(0);
    pub fn px(value: f32) -> Self {
        if value.is_finite() && value != 0. {
            Self(value.clamp(-1_000_000., 1_000_000.).to_bits())
        } else {
            Self::ZERO
        }
    }
    pub fn pixels(self) -> f32 {
        f32::from_bits(self.0)
    }
}
/// Physical paragraph alignment within an allocated text width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlign {
    #[default]
    Start,
    Left,
    Center,
    Right,
}
/// Soft wrapping policy when text has an available width. Explicit nowrap takes priority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WordWrap {
    /// Break an oversized word between graphemes, preserving the native default.
    #[default]
    WordOrGlyph,
    /// Break only at word boundaries; an oversized word stays on one line.
    Word,
}
/// Display-only overflow policy. Editor documents are never truncated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextOptions {
    pub overflow: TextOverflow,
    pub line_clamp: Option<std::num::NonZeroU32>,
}
/// Canonical OpenType feature settings. Tags use four ASCII bytes such as
/// `*b"liga"`; the last occurrence of a tag wins. Sorted storage gives equivalent
/// declarations identical font-cache keys. Unsupported tags are ignored by fonts.
type FeatureSettings = std::sync::Arc<[([u8; 4], u32)]>;
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontFeatures(Option<FeatureSettings>);
impl FontFeatures {
    pub fn new(features: impl IntoIterator<Item = ([u8; 4], u32)>) -> Self {
        let mut canonical = std::collections::BTreeMap::new();
        for (tag, value) in features {
            canonical.insert(tag, value);
        }
        if canonical.is_empty() {
            Self::default()
        } else {
            Self(Some(canonical.into_iter().collect::<Vec<_>>().into()))
        }
    }
    pub fn settings(&self) -> &[([u8; 4], u32)] {
        self.0.as_deref().unwrap_or(&[])
    }
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of_val(self.settings())
    }
}
/// Explicit finite OpenType design coordinates. This type carries typography,
/// never device scaling. Duplicate tags use the last value; order is canonical.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontVariations(Option<std::sync::Arc<[([u8; 4], u32)]>>);
impl FontVariations {
    pub fn new(values: impl IntoIterator<Item = ([u8; 4], f32)>) -> Option<Self> {
        let mut canonical = std::collections::BTreeMap::new();
        for (tag, value) in values {
            if !value.is_finite() || !tag.iter().all(|v| (0x20..=0x7e).contains(v)) {
                return None;
            }
            canonical.insert(tag, if value == 0. { 0 } else { value.to_bits() });
        }
        Some(if canonical.is_empty() {
            Self::default()
        } else {
            Self(Some(canonical.into_iter().collect::<Vec<_>>().into()))
        })
    }
    pub fn settings(&self) -> impl Iterator<Item = ([u8; 4], f32)> + '_ {
        self.0
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|(tag, bits)| (*tag, f32::from_bits(*bits)))
    }
    pub fn storage_bytes(&self) -> usize {
        std::mem::size_of_val(self.0.as_deref().unwrap_or(&[]))
    }
}
/// Optical sizing is opt-in for native clients. The CSS bridge selects Auto for
/// CSS's initial value, resolving logical font pixels before device scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontOpticalSizing {
    #[default]
    None,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontStyle {
    pub family: FontFamily,
    /// OpenType weight in the inclusive range 1..=1000 (400 regular, 700 bold).
    pub weight: u16,
    pub italic: bool,
    pub line_height: LineHeight,
    pub letter_spacing: LetterSpacing,
    pub features: FontFeatures,
    pub variations: FontVariations,
    pub optical_sizing: FontOpticalSizing,
    /// Inherited paint only; excluded from shape and measurement keys.
    pub text_shadows: crate::text_shadow::TextShadows,
    pub align: TextAlign,
    /// Rich paragraphs use the first run's policy for all their lines.
    pub word_wrap: WordWrap,
    /// Explicit ordered families tried before platform fallback for missing glyphs.
    pub fallbacks: std::sync::Arc<[FontFamily]>,
}
impl Default for FontStyle {
    fn default() -> Self {
        Self {
            family: FontFamily::SansSerif,
            weight: 400,
            italic: false,
            line_height: LineHeight::NORMAL,
            letter_spacing: LetterSpacing::ZERO,
            features: FontFeatures::default(),
            variations: FontVariations::default(),
            optical_sizing: FontOpticalSizing::None,
            text_shadows: crate::text_shadow::TextShadows::NONE,
            align: TextAlign::Start,
            word_wrap: WordWrap::default(),
            fallbacks: {
                static EMPTY: std::sync::OnceLock<std::sync::Arc<[FontFamily]>> =
                    std::sync::OnceLock::new();
                EMPTY.get_or_init(|| std::sync::Arc::from([])).clone()
            },
        }
    }
}
impl FontStyle {
    pub fn shaping_style(&self) -> Self {
        Self { text_shadows: crate::text_shadow::TextShadows::NONE, ..self.clone() }
    }
    pub fn same_shape(&self, other: &Self) -> bool { self.shaping_style() == other.shaping_style() }
    /// Owned key payload, conservatively charging shared allocations per entry.
    pub fn storage_bytes(&self) -> usize {
        self.features
            .storage_bytes()
            .saturating_add(self.text_shadows.storage_bytes())
            .saturating_add(self.variations.storage_bytes())
            .saturating_add(std::mem::size_of_val(self.fallbacks.as_ref()))
            .saturating_add(
                self.fallbacks
                    .iter()
                    .map(|family| match family {
                        FontFamily::Named(name) => name.len(),
                        _ => 0,
                    })
                    .sum::<usize>(),
            )
            .saturating_add(match &self.family {
                FontFamily::Named(name) => name.len(),
                _ => 0,
            })
    }
}
/// Font-aware shaping callback. The legacy TextShaper remains available for default fonts.
pub trait FontTextShaper {
    fn shape(
        &self,
        text: &str,
        size: f32,
        width: Option<f32>,
        font: &FontStyle,
    ) -> Box<dyn TextLayout>;
}
impl<F: Fn(&str, f32, Option<f32>, &FontStyle) -> Box<dyn TextLayout>> FontTextShaper for F {
    fn shape(
        &self,
        text: &str,
        size: f32,
        width: Option<f32>,
        font: &FontStyle,
    ) -> Box<dyn TextLayout> {
        self(text, size, width, font)
    }
}
/// Which side of a shared cluster or soft-wrap boundary owns a caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAffinity {
    /// Trailing edge of the preceding logical cluster; upstream at a soft wrap.
    Before,
    /// Leading edge of the following logical cluster; downstream at a soft wrap.
    #[default]
    After,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextPosition {
    /// UTF-8 byte offset on a grapheme boundary.
    pub byte_offset: usize,
    pub affinity: TextAffinity,
}
pub trait TextLayout {
    /// Estimated retained bytes for cache admission, including this layout and
    /// its owned allocations. Shared font databases are excluded. This is cache
    /// accounting, not allocator usage or a bound on process RSS; caches must
    /// also charge their keys and entry overhead. Return `None` when no useful
    /// estimate is available: unknown custom layouts must not be retained by a
    /// byte-budgeted cache. An estimate must remain conservative if geometry
    /// queries use interior caches; return `None` when their growth is unknown.
    /// The default preserves existing custom shapers.
    fn cache_weight(&self) -> Option<usize> {
        None
    }
    fn size(&self) -> (f32, f32);
    /// Returns a UTF-8 byte offset on a grapheme boundary.
    fn hit_test(&self, x: f32, y: f32) -> usize;
    fn caret(&self, byte_offset: usize) -> Rect;
    /// Visual spans may be disjoint for bidirectional text.
    fn selection(&self, range: Range<usize>) -> Vec<Rect>;
    /// Affinity-aware geometry; defaults preserve compatibility with custom shapers.
    fn hit_position(&self, x: f32, y: f32) -> TextPosition {
        TextPosition {
            byte_offset: self.hit_test(x, y),
            affinity: TextAffinity::After,
        }
    }
    fn caret_position(&self, position: TextPosition) -> Rect {
        self.caret(position.byte_offset)
    }
    /// Logical byte-ordered range of the caret's visual row, excluding hard
    /// line endings. Native bidi layouts override this: physical left/right
    /// edges need not enclose every logical cluster on a mixed-direction row.
    fn visual_line_range(&self, position: TextPosition) -> (TextPosition, TextPosition) {
        let left = self.visual_line_edge(position, false);
        let right = self.visual_line_edge(position, true);
        if left.byte_offset <= right.byte_offset {
            (left, right)
        } else {
            (right, left)
        }
    }
    /// Physical left/right edge of the caret's visual line, excluding line endings.
    fn visual_line_edge(&self, position: TextPosition, end: bool) -> TextPosition {
        let caret = self.caret_position(position);
        self.hit_position(
            if end { f32::MAX } else { -f32::MAX },
            caret.y + caret.height * 0.5,
        )
    }
}
pub trait TextShaper {
    fn shape(&self, text: &str, font_size: f32, max_width: Option<f32>) -> Box<dyn TextLayout>;
}
impl<F: Fn(&str, f32, Option<f32>) -> Box<dyn TextLayout>> TextShaper for F {
    fn shape(&self, text: &str, font_size: f32, max_width: Option<f32>) -> Box<dyn TextLayout> {
        self(text, font_size, max_width)
    }
}
/// Headless word widths use Unicode word bounds; native hosts use full line-break shaping.
fn fallback_word_width<'a>(
    words: &mut std::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
    index: usize,
    mut advance: impl FnMut(usize) -> f32,
) -> Option<f32> {
    while words
        .peek()
        .is_some_and(|(start, word)| start + word.len() <= index)
    {
        words.next();
    }
    let &(start, word) = words.peek()?;
    if start != index || word.chars().all(char::is_whitespace) {
        return None;
    }
    Some(
        word.grapheme_indices(true)
            .map(|(offset, _)| advance(start + offset))
            .sum(),
    )
}
/// The same soft-boundary policy for rich fallback cells and size-only metrics.
/// Single-line ellipsis retains the existing glyph-level truncation contract.
pub(crate) fn fallback_rich_wrap_advance<'a>(
    rich: &'a crate::rich_text::RichText,
    words: &mut std::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
    index: usize,
    run_index: usize,
) -> Option<f32> {
    let advance = |run: &crate::rich_text::TextRun| {
        (run.font_size * 0.6 + run.font.letter_spacing.pixels()).max(0.)
    };
    let word_only = rich.runs()[0].font.word_wrap == WordWrap::Word
        && !(rich.options().overflow == TextOverflow::Ellipsis
            && rich
                .options()
                .line_clamp
                .is_none_or(|limit| limit.get() == 1));
    if !word_only {
        return Some(advance(&rich.runs()[run_index]));
    }
    let mut word_run = run_index;
    fallback_word_width(words, index, |byte| {
        while rich.runs()[word_run].range.end <= byte {
            word_run += 1;
        }
        advance(&rich.runs()[word_run])
    })
}

/// Approximate grapheme cells for headless/core use. Native hosts install a real shaper.
pub struct FallbackTextLayout {
    cells: Vec<(Range<usize>, Rect)>,
    end: Rect,
    size: (f32, f32),
}
impl FallbackTextLayout {
    /// Mixed-size approximate cells used when no native rich shaper is installed.
    pub fn with_rich(rich: &crate::rich_text::RichText, width: Option<f32>) -> Self {
        let mut cells: Vec<(Range<usize>, Rect)> = Vec::new();
        let (mut x, mut y, mut max_x, mut pitch) = (0_f32, 0_f32, 0_f32, 0_f32);
        let mut row_start = 0;
        let mut run_index = 0;
        let mut words = rich.text().split_word_bound_indices().peekable();
        for (index, grapheme) in rich.text().grapheme_indices(true) {
            while rich.runs()[run_index].range.end <= index {
                run_index += 1;
            }
            let run = &rich.runs()[run_index];
            let line = run.font.line_height.resolve(run.font_size);
            let newline = grapheme.contains(['\r', '\n']);
            let advance = (run.font_size * 0.6 + run.font.letter_spacing.pixels()).max(0.);
            let wrapping_advance = fallback_rich_wrap_advance(rich, &mut words, index, run_index);
            if newline
                || width
                    .is_some_and(|w| x > 0. && wrapping_advance.is_some_and(|a| x + a > w.max(0.)))
            {
                if newline {
                    pitch = pitch.max(line);
                }
                for (_, rect) in &mut cells[row_start..] {
                    rect.height = pitch;
                }
                max_x = max_x.max(x);
                y += pitch;
                x = 0.;
                pitch = 0.;
                row_start = cells.len();
                if newline {
                    pitch = line;
                    continue;
                }
            }
            pitch = pitch.max(line);
            cells.push((
                index..index + grapheme.len(),
                Rect::new(x, y, advance, pitch),
            ));
            x += advance;
        }
        for (_, rect) in &mut cells[row_start..] {
            rect.height = pitch;
        }
        let mut layout = Self {
            cells,
            end: Rect::new(x, y, 1., pitch),
            size: (max_x.max(x), y + pitch),
        };
        if let Some(limit) = rich.options().line_clamp.map(|n| n.get()).or((rich
            .options()
            .overflow
            == TextOverflow::Ellipsis)
            .then_some(1))
        {
            let mut row = 0;
            let mut y = None;
            let mut keep = 0;
            for (_, rect) in &layout.cells {
                if y != Some(rect.y) {
                    row += 1;
                    y = Some(rect.y);
                }
                if row > limit {
                    break;
                }
                keep += 1;
            }
            let hidden = keep < layout.cells.len();
            layout.cells.truncate(keep);
            if hidden
                && rich.options().overflow == TextOverflow::Ellipsis
                && let Some(width) = width
            {
                let last = layout
                    .cells
                    .last()
                    .map(|(range, rect)| (range.start, rect.y));
                if let Some((index, y)) = last {
                    let advance = rich
                        .runs()
                        .iter()
                        .find(|r| r.range.contains(&index))
                        .map_or(0., |r| {
                            (r.font_size * 0.6 + r.font.letter_spacing.pixels()).max(0.)
                        });
                    while layout
                        .cells
                        .last()
                        .is_some_and(|(_, r)| r.y == y && r.x + r.width + advance > width)
                    {
                        layout.cells.pop();
                    }
                }
            }
            if let Some((_, rect)) = layout.cells.last() {
                layout.end = Rect::new(rect.x + rect.width, rect.y, 1., rect.height);
            }
            layout.size = crate::rich_text::fallback_measure(rich, width);
        }
        layout.align(
            width,
            rich.runs()
                .first()
                .map_or(TextAlign::Left, |r| r.font.align),
        );
        layout
    }
    pub fn new(text: &str, size: f32, max_width: Option<f32>) -> Self {
        Self::with_line_height(text, size, max_width, LineHeight::NORMAL)
    }
    pub fn with_line_height(
        text: &str,
        size: f32,
        max_width: Option<f32>,
        line_height: LineHeight,
    ) -> Self {
        Self::with_font(
            text,
            size,
            max_width,
            &FontStyle {
                line_height,
                ..FontStyle::default()
            },
        )
    }
    /// Grapheme-cell fallback with the same metrics as font-aware measurement.
    /// Tracking includes the final cell advance. Extreme negative tracking
    /// collapses advances to zero; native glyph shaping may overlap instead.
    pub fn with_font(text: &str, size: f32, max_width: Option<f32>, font: &FontStyle) -> Self {
        let mut cells = Vec::new();
        let (end, size) = Self::place(text, size, max_width, font, |range, rect| {
            cells.push((range, rect))
        });
        let mut layout = Self { cells, end, size };
        layout.align(max_width, font.align);
        layout
    }
    fn align(&mut self, width: Option<f32>, alignment: TextAlign) {
        let Some(width) = width.filter(|w| w.is_finite() && *w > 0.) else {
            return;
        };
        let factor = match alignment {
            TextAlign::Start | TextAlign::Left => return,
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.,
        };
        let mut start = 0;
        while start < self.cells.len() {
            let y = self.cells[start].1.y;
            let mut end = start + 1;
            while end < self.cells.len() && self.cells[end].1.y == y {
                end += 1;
            }
            let row_width = self.cells[start..end]
                .iter()
                .map(|(_, r)| r.x + r.width)
                .fold(0_f32, f32::max);
            let shift = (width - row_width).max(0.) * factor;
            for (_, rect) in &mut self.cells[start..end] {
                rect.x += shift;
            }
            if self.end.y == y {
                self.end.x += shift;
            }
            start = end;
        }
        if self.cells.last().is_none_or(|(_, r)| r.y != self.end.y) {
            self.end.x += width * factor;
        }
    }
    /// Measurement shares shaping's grapheme placement with O(1) auxiliary memory.
    pub fn measure(
        text: &str,
        size: f32,
        max_width: Option<f32>,
        line_height: LineHeight,
    ) -> (f32, f32) {
        Self::measure_with_font(
            text,
            size,
            max_width,
            &FontStyle {
                line_height,
                ..FontStyle::default()
            },
        )
    }
    pub fn measure_with_font(
        text: &str,
        size: f32,
        max_width: Option<f32>,
        font: &FontStyle,
    ) -> (f32, f32) {
        Self::place(text, size, max_width, font, |_, _| {}).1
    }
    fn place(
        text: &str,
        size: f32,
        max_width: Option<f32>,
        font: &FontStyle,
        mut cell: impl FnMut(Range<usize>, Rect),
    ) -> (Rect, (f32, f32)) {
        let size = if size.is_finite() && size > 0. { size } else { 1. };
        // Extreme negative tracking collapses cells rather than reversing their
        // order. Include the final advance, consistently with native tracking.
        let advance = (size as f64 * 0.6 + font.letter_spacing.pixels() as f64)
            .clamp(0., f32::MAX as f64) as f32;
        let height = font.line_height.resolve(size);
        let mut x = 0.;
        let mut y = 0.;
        let mut width = 0_f32;
        let mut words = text.split_word_bound_indices().peekable();
        for (index, grapheme) in text.grapheme_indices(true) {
            if grapheme.contains(['\n', '\r']) {
                cell(index..index + grapheme.len(), Rect::new(x, y, 0., height));
                width = width.max(x);
                x = 0.;
                y = (y + height).min(f32::MAX);
                continue;
            }
            let wrapping_advance = if font.word_wrap == WordWrap::Word {
                fallback_word_width(&mut words, index, |_| advance)
            } else {
                Some(advance)
            };
            if max_width.is_some_and(|w| {
                (w > 0. || font.word_wrap == WordWrap::Word)
                    && x > 0.
                    && wrapping_advance.is_some_and(|a| x + a > w.max(0.))
            }) {
                width = width.max(x);
                x = 0.;
                y = (y + height).min(f32::MAX)
            }
            cell(
                index..index + grapheme.len(),
                Rect::new(x, y, advance, height),
            );
            x = (x + advance).min(f32::MAX);
        }
        width = width.max(x);
        (
            Rect::new(x, y, 1., height),
            (width, (y + height).min(f32::MAX)),
        )
    }
}
impl TextLayout for FallbackTextLayout {
    fn cache_weight(&self) -> Option<usize> {
        Some(
            std::mem::size_of::<Self>().saturating_add(
                self.cells
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(Range<usize>, Rect)>()),
            ),
        )
    }
    fn size(&self) -> (f32, f32) {
        self.size
    }
    fn hit_test(&self, x: f32, y: f32) -> usize {
        self.hit_position(x, y).byte_offset
    }
    fn hit_position(&self, x: f32, y: f32) -> TextPosition {
        let y = y.max(0.);
        let mut result = TextPosition::default();
        for (range, r) in &self.cells {
            if y >= r.y && y < r.y + r.height {
                if r.width == 0. || x < r.x + r.width * 0.5 {
                    return TextPosition {
                        byte_offset: range.start,
                        affinity: TextAffinity::After,
                    };
                }
                result = TextPosition {
                    byte_offset: range.end,
                    affinity: TextAffinity::Before,
                };
            } else if y >= r.y + r.height {
                result = TextPosition {
                    byte_offset: range.end,
                    affinity: TextAffinity::After,
                };
            }
        }
        result
    }
    fn caret_position(&self, position: TextPosition) -> Rect {
        if position.affinity == TextAffinity::Before {
            for (range, rect) in &self.cells {
                if range.end == position.byte_offset && rect.width > 0. {
                    return Rect::new(rect.x + rect.width, rect.y, 1., rect.height);
                }
            }
        }
        self.caret(position.byte_offset)
    }
    fn caret(&self, byte_offset: usize) -> Rect {
        for (range, r) in &self.cells {
            if byte_offset <= range.start || range.contains(&byte_offset) {
                return Rect::new(r.x, r.y, 1., r.height);
            }
        }
        self.end
    }
    fn selection(&self, range: Range<usize>) -> Vec<Rect> {
        let start = range.start.min(range.end);
        let end = range.start.max(range.end);
        if start == end {
            return Vec::new();
        }
        let mut result: Vec<Rect> = Vec::new();
        for (index, r) in &self.cells {
            if index.start < end && index.end > start && r.width > 0. {
                if let Some(last) = result.last_mut()
                    && last.y == r.y
                    && last.x + last.width >= r.x
                {
                    *last = last.union(*r);
                    continue;
                }
                result.push(*r);
            }
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_height_normalization_hashing_and_tight_pitch() {
        use std::collections::HashSet;
        let invalid = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1., -0., 0.];
        let mut heights: HashSet<_> = invalid.into_iter().map(LineHeight::px).collect();
        assert_eq!(heights.len(), 1);
        assert!(heights.contains(&LineHeight::NORMAL));
        assert_eq!(LineHeight::NORMAL.resolve(10.), 14.);
        assert_eq!(LineHeight::NORMAL.resolve(11.), 16.);
        assert_eq!(LineHeight::NORMAL.resolve(f32::MAX), f32::MAX);
        assert!(LineHeight::NORMAL.resolve(f32::INFINITY).is_finite());
        heights.insert(LineHeight::px(14.));
        assert_eq!(
            heights.len(),
            2,
            "normal remains size-dependent, unlike explicit14px"
        );
        assert_eq!(LineHeight::px(0.5).resolve(40.), 0.5);
        assert_eq!(
            LineHeight::px(f32::from_bits(1)).pixels(),
            Some(f32::from_bits(1))
        );
    }
    #[test]
    fn subpixel_fallback_and_normal_pitch_keep_positive_sizes() {
        for size in [0.125_f32, 0.7475, 1., 13.] {
            let layout = FallbackTextLayout::new("MMMM", size, None);
            assert!((layout.size().0 - size * 0.6 * 4.).abs() < 0.00001);
            assert_eq!(layout.size().1, (size * 1.4).ceil());
        }
        for size in [f32::from_bits(1), f32::MIN_POSITIVE] {
            assert_eq!(LineHeight::NORMAL.resolve(size), 1.);
        }
        let unit = FallbackTextLayout::new("MMMM", 1., None).size();
        for size in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1., -0., 0.] {
            assert_eq!(FallbackTextLayout::new("MMMM", size, None).size(), unit);
        }
    }
    #[test]
    fn rounded_pitch_preserves_natural_baseline_and_cache_identity() {
        let natural = LineHeight::rounded_px(16.25);
        assert_eq!(natural.resolve(12.5), 16.);
        assert_eq!(natural.baseline_offset(), 0.125);
        assert_ne!(natural, LineHeight::px(16.));
        assert_eq!(LineHeight::rounded_px(17.), LineHeight::px(17.));
        assert_eq!(LineHeight::px(16.).baseline_offset(), 0.);
        assert_eq!(LineHeight::NORMAL.baseline_offset(), 0.);
        assert_eq!(LineHeight::rounded_px(0.25).resolve(10.), 1.);
        for value in [f32::NAN, f32::INFINITY, -1., 0.] {
            assert_eq!(LineHeight::rounded_px(value), LineHeight::NORMAL);
        }
        let layout = FallbackTextLayout::with_line_height("a\nb", 12.5, None, natural);
        assert_eq!(layout.size().1, 32.);
        assert_eq!(layout.caret(2).y, 16.);
    }
    #[test]
    fn word_only_fallback_keeps_long_words_and_matches_rich_measurement() {
        use crate::rich_text::{RichText, TextRun};
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            ..Default::default()
        };
        let value = "Active Paused";
        let layout = FallbackTextLayout::with_font(value, 10., Some(35.99), &font);
        assert_eq!(layout.size(), (42., 28.));
        assert_eq!(layout.caret(5).y, 0.);
        assert_eq!(layout.caret(7).y, 14.);
        assert_eq!(
            layout.size(),
            FallbackTextLayout::measure_with_font(value, 10., Some(35.99), &font)
        );
        let rich = RichText::new(
            value,
            vec![
                TextRun {
                    range: 0..3,
                    font: font.clone(),
                    font_size: 10.,
                    ..Default::default()
                },
                TextRun {
                    range: 3..value.len(),
                    font: FontStyle::default(),
                    font_size: 10.,
                    ..Default::default()
                },
            ],
        )
        .unwrap();
        assert_eq!(
            FallbackTextLayout::with_rich(&rich, Some(35.99)).size(),
            layout.size()
        );
        assert_eq!(
            crate::rich_text::fallback_measure(&rich, Some(35.99)),
            layout.size()
        );
        assert_eq!(
            FallbackTextLayout::with_font("Active", 10., Some(35.99), &font).size(),
            (36., 14.)
        );
        assert_eq!(
            FallbackTextLayout::new("Active", 10., Some(35.99)).size().1,
            28.
        );
        assert_eq!(
            FallbackTextLayout::with_font(value, 10., None, &font)
                .size()
                .1,
            14.
        );
    }
    #[test]
    fn rich_word_fallback_clamp_and_ellipsis_share_finite_metrics() {
        use crate::rich_text::{RichText, TextRun, fallback_measure};
        use std::num::NonZeroU32;
        let value = "ab cd ef";
        let font = FontStyle {
            word_wrap: WordWrap::Word,
            line_height: LineHeight::px(20.),
            ..Default::default()
        };
        for (overflow, clamp, expected, cells) in [
            (TextOverflow::Clip, None, (18., 60.), 8),
            (TextOverflow::Clip, Some(1), (18., 20.), 3),
            (TextOverflow::Clip, Some(2), (18., 40.), 6),
            (TextOverflow::Ellipsis, None, (18., 20.), 2),
            (TextOverflow::Ellipsis, Some(1), (18., 20.), 2),
            (TextOverflow::Ellipsis, Some(2), (18., 40.), 5),
        ] {
            let rich = RichText::new(
                value,
                vec![TextRun {
                    range: 0..value.len(),
                    font: font.clone(),
                    font_size: 10.,
                    ..Default::default()
                }],
            )
            .unwrap()
            .with_options(TextOptions {
                overflow,
                line_clamp: clamp.and_then(NonZeroU32::new),
            });
            // Both entry points previously recursed when Word met display options.
            assert_eq!(fallback_measure(&rich, Some(18.)), expected);
            assert_eq!(
                crate::rich_text::TextMeasure::default().measure(&rich, Some(18.)),
                expected
            );
            let layout = FallbackTextLayout::with_rich(&rich, Some(18.));
            assert_eq!(layout.size(), expected);
            assert_eq!(layout.cells.len(), cells);
            assert_eq!(fallback_measure(&rich, None), (48., 20.));
            assert_eq!(
                FallbackTextLayout::with_rich(&rich, None).size(),
                (48., 20.)
            );
        }
        let rich = RichText::new(
            "Active",
            vec![TextRun {
                range: 0..6,
                font,
                font_size: 10.,
                ..Default::default()
            }],
        )
        .unwrap()
        .with_options(TextOptions {
            line_clamp: NonZeroU32::new(1),
            ..Default::default()
        });
        assert_eq!(fallback_measure(&rich, Some(35.99)), (36., 20.));
        assert_eq!(
            FallbackTextLayout::with_rich(&rich, Some(35.99)).size(),
            (36., 20.)
        );
    }

    #[test]
    fn rich_word_fallback_preserves_empty_trailing_newline_and_mixed_pitch_heights() {
        use crate::rich_text::{RichText, TextRun, fallback_measure};
        use std::num::NonZeroU32;
        for policy in [WordWrap::Word, WordWrap::WordOrGlyph] {
            let font = FontStyle {
                word_wrap: policy,
                line_height: LineHeight::px(20.),
                ..Default::default()
            };
            for (value, natural, clamped) in [
                ("", (0., 0.), (0., 0.)),
                ("\n", (0., 40.), (0., 20.)),
                ("\r\n", (0., 40.), (0., 20.)),
                ("ab\n", (12., 40.), (12., 20.)),
                ("ab\r\n", (12., 40.), (12., 20.)),
            ] {
                for (clamp, expected) in [(None, natural), (Some(1), clamped), (Some(2), natural)] {
                    let runs = if value.is_empty() {
                        Vec::new()
                    } else {
                        vec![TextRun {
                            range: 0..value.len(),
                            font: font.clone(),
                            font_size: 10.,
                            ..Default::default()
                        }]
                    };
                    let rich = RichText::new(value, runs)
                        .unwrap()
                        .with_options(TextOptions {
                            line_clamp: clamp.and_then(NonZeroU32::new),
                            ..Default::default()
                        });
                    assert_eq!(
                        fallback_measure(&rich, None),
                        expected,
                        "{policy:?} {value:?} {clamp:?}"
                    );
                    assert_eq!(FallbackTextLayout::with_rich(&rich, None).size(), expected);
                }
            }
            let rich = RichText::new(
                "ab\ncd\n",
                vec![
                    TextRun {
                        range: 0..3,
                        font: font.clone(),
                        font_size: 10.,
                        ..Default::default()
                    },
                    TextRun {
                        range: 3..6,
                        font: FontStyle {
                            line_height: LineHeight::px(40.),
                            ..font
                        },
                        font_size: 20.,
                        ..Default::default()
                    },
                ],
            )
            .unwrap();
            for (clamp, expected) in [
                (None, (24., 100.)),
                (Some(1), (12., 20.)),
                (Some(2), (24., 60.)),
                (Some(3), (24., 100.)),
            ] {
                let rich = rich.clone().with_options(TextOptions {
                    line_clamp: clamp.and_then(NonZeroU32::new),
                    ..Default::default()
                });
                assert_eq!(fallback_measure(&rich, None), expected);
                assert_eq!(FallbackTextLayout::with_rich(&rich, None).size(), expected);
            }
        }
    }

    #[test]
    fn explicit_pitch_keeps_wrapped_unicode_hit_selection_and_caret_aligned() {
        let layout = FallbackTextLayout::with_line_height(
            "a👩‍💻e\u{301}z\n",
            10.,
            Some(12.),
            LineHeight::px(7.5),
        );
        assert_eq!(layout.size(), (12., 22.5));
        assert_eq!(layout.caret(12), Rect::new(0., 7.5, 1., 7.5));
        assert_eq!(layout.hit_test(1., 8.), 12);
        assert_eq!(layout.caret(17), Rect::new(0., 15., 1., 7.5));
        assert_eq!(
            layout.selection(1..15),
            vec![Rect::new(6., 0., 6., 7.5), Rect::new(0., 7.5, 6., 7.5)]
        );
    }
    #[test]
    fn measurement_and_shape_agree_for_unbounded_and_tiny_widths() {
        for width in [
            None,
            Some(-1.),
            Some(0.),
            Some(f32::NAN),
            Some(f32::from_bits(1)),
            Some(6.),
        ] {
            for pitch in [LineHeight::NORMAL, LineHeight::px(0.5), LineHeight::px(30.)] {
                let text = "a👩‍💻e\u{301}\r\nz\n";
                let measured = FallbackTextLayout::measure(text, 10., width, pitch);
                assert_eq!(
                    measured,
                    FallbackTextLayout::with_line_height(text, 10., width, pitch).size()
                );
                assert!(measured.0 > 0. && measured.1 > 0. && measured.1.is_finite());
            }
        }
    }
    #[test]
    fn grapheme_offsets_newlines_and_wrap() {
        let layout = FallbackTextLayout::new("e\u{301}👩‍💻\nx", 10., None);
        assert_eq!(layout.hit_test(1., 2.), 0);
        assert_eq!(layout.hit_test(8., 2.), 3);
        assert_eq!(layout.caret(3).x, 6.);
        assert_eq!(layout.selection(0..3).len(), 1);
        assert_eq!(layout.size().1, 28.);
    }
    #[test]
    fn wrapped_grapheme_boundary_preserves_visual_affinity() {
        let layout = FallbackTextLayout::new("a👩‍💻e\u{301}z", 10., Some(12.));
        let end = layout.visual_line_edge(TextPosition::default(), true);
        assert_eq!(
            end,
            TextPosition {
                byte_offset: 12,
                affinity: TextAffinity::Before
            }
        );
        assert_eq!(layout.caret_position(end), Rect::new(12., 0., 1., 14.));
        let next = TextPosition {
            affinity: TextAffinity::After,
            ..end
        };
        assert_eq!(layout.caret_position(next), Rect::new(0., 14., 1., 14.));
        assert_eq!(layout.visual_line_edge(next, false), next);
        assert_eq!(layout.hit_position(100., 5.), end);
        assert_eq!(layout.hit_position(-10., 18.), next);
        assert_eq!(
            layout.selection(1..15),
            vec![Rect::new(6., 0., 6., 14.), Rect::new(0., 14., 6., 14.)]
        );
        assert!(layout.selection(2..2).is_empty());
    }
    #[test]
    fn visual_edges_exclude_crlf_and_preserve_empty_final_lines() {
        let layout = FallbackTextLayout::new("ab\r\n\n", 10., Some(30.));
        assert_eq!(layout.hit_test(100., 2.), 2);
        assert_eq!(layout.hit_test(100., 16.), 4);
        assert_eq!(layout.hit_test(100., 30.), 5);
        assert_eq!(layout.caret(5).y, 28.);
        assert_eq!(layout.hit_test(10., -10.), 2);
    }
}

#[cfg(test)]
mod cache_weight_tests {
    use super::*;

    #[test]
    fn fallback_charges_spare_cell_capacity_without_counting_shared_sources() {
        let mut layout = FallbackTextLayout::new("abc", 14., None);
        let initial = layout.cache_weight().unwrap();
        layout.cells.reserve(1024);
        let expected = std::mem::size_of::<FallbackTextLayout>()
            + layout.cells.capacity() * std::mem::size_of::<(Range<usize>, Rect)>();
        assert_eq!(layout.cache_weight(), Some(expected));
        assert!(expected > initial);
    }

    #[test]
    fn existing_custom_layouts_have_unknown_weight_by_default() {
        struct Custom;
        impl TextLayout for Custom {
            fn size(&self) -> (f32, f32) {
                (0., 0.)
            }
            fn hit_test(&self, _: f32, _: f32) -> usize {
                0
            }
            fn caret(&self, _: usize) -> Rect {
                Rect::default()
            }
            fn selection(&self, _: Range<usize>) -> Vec<Rect> {
                Vec::new()
            }
        }
        assert_eq!(Custom.cache_weight(), None);
    }
}

#[cfg(test)]
mod letter_spacing_tests {
    use super::*;

    #[test]
    fn normalized_spacing_has_stable_equality_and_hash() {
        use std::collections::HashSet;
        let normalized: HashSet<_> = [0., -0., f32::NAN, f32::INFINITY, f32::NEG_INFINITY]
            .into_iter()
            .map(LetterSpacing::px)
            .collect();
        assert_eq!(normalized.len(), 1);
        assert_eq!(LetterSpacing::px(-2.).pixels(), -2.);
        assert_eq!(LetterSpacing::px(2.).pixels(), 2.);
        assert_eq!(LetterSpacing::px(f32::MAX), LetterSpacing::px(1_000_000.));
        assert_eq!(LetterSpacing::px(-f32::MAX), LetterSpacing::px(-1_000_000.));
        assert_eq!(LetterSpacing::px(999_999.).pixels(), 999_999.);
        assert_eq!(LetterSpacing::px(-999_999.).pixels(), -999_999.);
    }

    #[test]
    fn spacing_changes_grapheme_advance_wrap_and_selection_consistently() {
        let font = FontStyle {
            letter_spacing: LetterSpacing::px(4.),
            ..FontStyle::default()
        };
        let text = "a\u{301}bc";
        let layout = FallbackTextLayout::with_font(text, 10., Some(20.), &font);
        assert_eq!(layout.size(), (20., 28.));
        assert_eq!(layout.caret(3), Rect::new(10., 0., 1., 14.));
        assert_eq!(layout.caret(4).y, 14.);
        assert_eq!(layout.hit_test(9., 7.), 3);
        assert_eq!(layout.selection(0..3), vec![Rect::new(0., 0., 10., 14.)]);
        assert_eq!(
            layout.size(),
            FallbackTextLayout::measure_with_font(text, 10., Some(20.), &font)
        );
        let negative = FontStyle {
            letter_spacing: LetterSpacing::px(-2.),
            ..font
        };
        assert_eq!(
            FallbackTextLayout::with_font(text, 10., None, &negative)
                .size()
                .0,
            12.
        );
    }

    #[test]
    fn extreme_spacing_keeps_fallback_geometry_finite_and_nonnegative() {
        for spacing in [-f32::MAX, f32::MAX] {
            let font = FontStyle {
                letter_spacing: LetterSpacing::px(spacing),
                ..FontStyle::default()
            };
            let layout = FallbackTextLayout::with_font("ab\ncd", 10., None, &font);
            let (w, h) = layout.size();
            assert!(w.is_finite() && h.is_finite() && w >= 0. && h >= 0.);
            let caret = layout.caret(5);
            assert!(caret.x.is_finite() && caret.x >= 0.);
            assert_eq!(
                layout.size(),
                FallbackTextLayout::measure_with_font("ab\ncd", 10., None, &font)
            );
        }
    }
}

#[cfg(test)]
mod font_variation_key_tests {
    use super::*;
    use std::hash::{Hash, Hasher};
    fn hash(value: &FontVariations) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut h);
        h.finish()
    }
    #[test]
    fn canonical_axes_preserve_values_and_reject_nonfinite_cache_keys() {
        let first =
            FontVariations::new([(*b"wght", 600.), (*b"opsz", 14.), (*b"opsz", 17.)]).unwrap();
        let same = FontVariations::new([(*b"opsz", 17.), (*b"wght", 600.)]).unwrap();
        assert_eq!(first, same);
        assert_eq!(hash(&first), hash(&same));
        assert_eq!(
            first.settings().collect::<Vec<_>>(),
            vec![(*b"opsz", 17.), (*b"wght", 600.)]
        );
        assert_ne!(
            same,
            FontVariations::new([(*b"opsz", 14.), (*b"wght", 600.)]).unwrap()
        );
        assert_eq!(
            FontVariations::new([(*b"slnt", -0.)]),
            FontVariations::new([(*b"slnt", 0.)])
        );
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(FontVariations::new([(*b"opsz", value)]).is_none());
        }
        assert!(FontVariations::new([(*b"a\0bc", 1.)]).is_none());
        assert_eq!(FontStyle::default().optical_sizing, FontOpticalSizing::None);
    }
}
