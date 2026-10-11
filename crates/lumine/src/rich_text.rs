//! Immutable display text with continuous UTF-8 styled runs.
use crate::{scene::Color, text_layout::FontStyle};
use std::{ops::Range, sync::Arc};
pub(crate) type RichShaper =
    dyn Fn(&RichText, Option<f32>) -> Box<dyn crate::text_layout::TextLayout>;
pub(crate) type RichMeasurer = dyn Fn(&RichText, Option<f32>) -> (f32, f32);

/// Measures rich text exactly as layout does: with the host's installed
/// measurers (prepared text on desktop, where a size at a new width costs line
/// breaking arithmetic), or the headless fallback until one is installed.
/// Apps use it for the heights of content they have not mounted, such as the
/// estimates of a virtualized list; hosts can measure those without keeping
/// glyphs for drawing (`Scene::set_detached_rich_text_measurer`).
#[derive(Clone, Default)]
pub struct TextMeasure(std::rc::Rc<std::cell::RefCell<Measurers>>);
#[derive(Default)]
struct Measurers {
    layout: Option<Box<RichMeasurer>>,
    detached: Option<Box<RichMeasurer>>,
}
impl TextMeasure {
    /// `rich`'s (width, height) wrapped at `width`, or unwrapped for `None`.
    pub fn measure(&self, rich: &RichText, width: Option<f32>) -> (f32, f32) {
        let measurers = self.0.borrow();
        match measurers.detached.as_ref().or(measurers.layout.as_ref()) {
            Some(measure) => measure(rich, width),
            None => fallback_measure(rich, width),
        }
    }
    /// Whether a host measurer is installed (else the headless fallback).
    pub(crate) fn is_native(&self) -> bool {
        self.0.borrow().layout.is_some()
    }
    /// For mounted text, which is drawn next.
    pub(crate) fn measure_mounted(&self, rich: &RichText, width: Option<f32>) -> (f32, f32) {
        match &self.0.borrow().layout {
            Some(measure) => measure(rich, width),
            None => fallback_measure(rich, width),
        }
    }
    pub(crate) fn install(&self, measure: Box<RichMeasurer>) {
        let previous = self.0.borrow_mut().layout.replace(measure);
        drop(previous);
    }
    pub(crate) fn install_detached(&self, measure: Box<RichMeasurer>) {
        let previous = self.0.borrow_mut().detached.replace(measure);
        drop(previous);
    }
}

/// A solid text decoration. An unspecified color uses the run foreground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoration {
    pub color: Option<Color>,
    thickness: f32,
}
impl Decoration {
    pub fn new(thickness: f32) -> Self {
        Self {
            color: None,
            thickness: if thickness.is_finite() && thickness > 0. {
                thickness
            } else {
                1.
            },
        }
    }
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
    pub fn thickness(self) -> f32 {
        self.thickness
    }
}
impl Default for Decoration {
    fn default() -> Self {
        Self::new(1.)
    }
}

/// One resolved style over a contiguous UTF-8 byte range.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub range: Range<usize>,
    pub font: FontStyle,
    pub font_size: f32,
    pub color: Color,
    pub background: Option<Color>,
    pub underline: Option<Decoration>,
    pub strikethrough: Option<Decoration>,
}

impl Default for TextRun {
    fn default() -> Self {
        Self {
            range: 0..0,
            font: Default::default(),
            font_size: 16.,
            color: Color(255, 255, 255, 255),
            background: None,
            underline: None,
            strikethrough: None,
        }
    }
}

/// Validated, immutable styled display text. Runs cover the entire string exactly;
/// empty strings use no runs. Adjacent styles may cross grapheme boundaries: the
/// native shaper still shapes the paragraph continuously.
#[derive(Clone, Debug, PartialEq)]
pub struct RichText {
    options: crate::text_layout::TextOptions,
    text: Arc<str>,
    runs: Arc<[TextRun]>,
}
impl RichText {
    pub fn new(
        text: impl Into<Arc<str>>,
        runs: impl Into<Arc<[TextRun]>>,
    ) -> Result<Self, &'static str> {
        let text = text.into();
        let runs = runs.into();
        let mut end = 0;
        for run in runs.iter() {
            if run.range.start != end
                || run.range.end <= end
                || run.range.end > text.len()
                || !text.is_char_boundary(run.range.start)
                || !text.is_char_boundary(run.range.end)
            {
                return Err("rich text runs must cover contiguous nonempty UTF-8 ranges");
            }
            if !run.font_size.is_finite() || run.font_size <= 0. {
                return Err("rich text font size must be finite and positive");
            }
            end = run.range.end;
        }
        if end != text.len() {
            return Err("rich text runs must cover the entire string");
        }
        Ok(Self {
            text,
            runs,
            options: Default::default(),
        })
    }
    pub fn with_options(mut self, options: crate::text_layout::TextOptions) -> Self {
        self.options = options;
        self
    }
    pub fn options(&self) -> crate::text_layout::TextOptions {
        self.options
    }
    /// Whether `other` differs from `self` only in colors: same text, runs,
    /// fonts and decoration presence, so shaped glyphs and decoration geometry
    /// can be reused and recoloured.
    pub fn same_shape(&self, other: &Self) -> bool {
        self.same_metrics(other)
            && self.runs.iter().zip(other.runs.iter()).all(|(a, b)| {
                a.background.is_some() == b.background.is_some()
                    && a.underline.map(|d| d.thickness()) == b.underline.map(|d| d.thickness())
                    && a.strikethrough.map(|d| d.thickness())
                        == b.strikethrough.map(|d| d.thickness())
            })
    }
    pub(crate) fn same_metrics(&self, other: &Self) -> bool {
        self.options == other.options
            && self.text == other.text
            && self.runs.len() == other.runs.len()
            && self
                .runs
                .iter()
                .zip(other.runs.iter())
                .all(|(a, b)| a.range == b.range && a.font == b.font && a.font_size == b.font_size)
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn text_arc(&self) -> Arc<str> {
        self.text.clone()
    }
    /// Conservative payload accounting for bounded caches; shared font names may
    /// be charged more than once. Excludes allocator and font-engine overhead.
    pub fn storage_bytes(&self) -> usize {
        self.text
            .len()
            .saturating_add(std::mem::size_of_val(self.runs.as_ref()))
            .saturating_add(
                self.runs
                    .iter()
                    .map(|run| run.font.storage_bytes())
                    .fold(0usize, usize::saturating_add),
            )
    }
    pub fn runs(&self) -> &[TextRun] {
        &self.runs
    }
}

/// Approximate, allocation-free headless metrics. Native hosts replace this with
/// full paragraph shaping; grapheme widths here follow the fallback text model.
pub fn fallback_measure(rich: &RichText, width: Option<f32>) -> (f32, f32) {
    use unicode_segmentation::UnicodeSegmentation;
    if rich.text.is_empty() {
        return (0., 0.);
    }
    let mut run_index = 0;
    let limit = rich
        .options
        .line_clamp
        .map(|n| n.get())
        .or((rich.options.overflow == crate::text_layout::TextOverflow::Ellipsis).then_some(1));
    let mut row = 1;
    let (mut x, mut y, mut max_x, mut pitch) = (0_f32, 0_f32, 0_f32, 0_f32);
    for (index, grapheme) in rich.text.grapheme_indices(true) {
        while rich.runs[run_index].range.end <= index {
            run_index += 1;
        }
        let run = &rich.runs[run_index];
        let line = run.font.line_height.resolve(run.font_size);
        if grapheme == "\n" || grapheme == "\r\n" || grapheme == "\r" {
            if limit.is_some_and(|limit| row >= limit) {
                return (
                    width.map_or(max_x.max(x), |w| {
                        max_x
                            .max(
                                x + if rich.options.overflow
                                    == crate::text_layout::TextOverflow::Ellipsis
                                {
                                    run.font_size * 0.6
                                } else {
                                    0.
                                },
                            )
                            .min(w.max(0.))
                    }),
                    y + pitch.max(line),
                );
            }
            row += 1;
            max_x = max_x.max(x);
            y += pitch.max(line);
            x = 0.;
            pitch = line;
            continue;
        }
        let advance = (run.font_size * 0.6 + run.font.letter_spacing.pixels()).max(0.);
        if width.is_some_and(|w| x > 0. && x + advance > w.max(0.)) {
            if limit.is_some_and(|limit| row >= limit) {
                return (
                    width.unwrap().max(0.).min(max_x.max(
                        x + if rich.options.overflow == crate::text_layout::TextOverflow::Ellipsis {
                            advance
                        } else {
                            0.
                        },
                    )),
                    y + pitch,
                );
            }
            row += 1;
            max_x = max_x.max(x);
            y += pitch;
            x = 0.;
            pitch = 0.;
        }
        pitch = pitch.max(line);
        x += advance;
    }
    (max_x.max(x), y + pitch)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_measurers_drops_captures_outside_the_shared_borrow() {
        use std::{
            cell::{Cell, RefCell},
            rc::{Rc, Weak},
        };
        struct Capture(Weak<RefCell<Measurers>>, Rc<Cell<f32>>);
        impl Drop for Capture {
            fn drop(&mut self) {
                if let Some(shared) = self.0.upgrade() {
                    self.1.set(
                        TextMeasure(shared)
                            .measure(&RichText::new("", Vec::new()).unwrap(), None)
                            .0,
                    );
                }
            }
        }
        for detached in [false, true] {
            let measure = TextMeasure::default();
            let observed = Rc::new(Cell::new(0.));
            let capture = Capture(Rc::downgrade(&measure.0), observed.clone());
            let first: Box<RichMeasurer> = Box::new(move |_, _| {
                let _ = &capture;
                (1., 1.)
            });
            if detached {
                measure.install_detached(first);
                measure.install_detached(Box::new(|_, _| (2., 2.)));
            } else {
                measure.install(first);
                measure.install(Box::new(|_, _| (2., 2.)));
            }
            assert_eq!(observed.get(), 2.);
        }
    }

    fn run(range: Range<usize>) -> TextRun {
        TextRun {
            range,
            font: Default::default(),
            font_size: 16.,
            color: Color(255, 255, 255, 255),
            ..Default::default()
        }
    }
    #[test]
    fn validates_unicode_coverage_sizes_and_empty_text() {
        assert!(RichText::new("é字", vec![run(0..2), run(2..5)]).is_ok());
        for ranges in [
            vec![0..1, 1..5],
            vec![0..2, 3..5],
            std::iter::once(0..2).collect(),
            vec![0..0, 0..5],
        ] {
            assert!(RichText::new("é字", ranges.into_iter().map(run).collect::<Vec<_>>()).is_err());
        }
        assert!(RichText::new("", Vec::new()).is_ok());
        for size in [0., -1., f32::NAN, f32::INFINITY] {
            let mut r = run(0..1);
            r.font_size = size;
            assert!(RichText::new("x", vec![r]).is_err());
        }
    }
}
