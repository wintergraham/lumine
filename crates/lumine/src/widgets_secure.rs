use crate::{
    scene::Rect,
    text_layout::{TextLayout, TextPosition},
};
use std::{ops::Range, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;

const BULLET: &str = "•";

/// Retains only masked text and source byte boundaries, never source characters.
pub(super) struct MaskedText {
    pub text: String,
    pub source_offsets: Rc<[usize]>,
}

impl MaskedText {
    pub fn new(source: &str) -> Self {
        let mut text = String::new();
        let mut offsets = Vec::new();
        for (offset, _) in source.grapheme_indices(true) {
            offsets.push(offset);
            text.push_str(BULLET);
        }
        offsets.push(source.len());
        Self {
            text,
            source_offsets: offsets.into(),
        }
    }

    pub fn display_offset(&self, source_offset: usize) -> usize {
        display_offset(&self.source_offsets, source_offset)
    }

    pub fn source_offset(&self, display_offset: usize) -> usize {
        source_offset(&self.source_offsets, display_offset)
    }
}

fn display_offset(offsets: &[usize], source: usize) -> usize {
    offsets
        .partition_point(|offset| *offset <= source)
        .saturating_sub(1)
        * BULLET.len()
}

fn source_offset(offsets: &[usize], display: usize) -> usize {
    offsets[(display / BULLET.len()).min(offsets.len() - 1)]
}

pub(super) struct MaskedLayout {
    pub layout: Box<dyn TextLayout>,
    pub source_offsets: Rc<[usize]>,
}

impl MaskedLayout {
    fn to_display(&self, position: TextPosition) -> TextPosition {
        TextPosition {
            byte_offset: display_offset(&self.source_offsets, position.byte_offset),
            ..position
        }
    }

    fn to_source(&self, position: TextPosition) -> TextPosition {
        TextPosition {
            byte_offset: source_offset(&self.source_offsets, position.byte_offset),
            ..position
        }
    }
}

impl TextLayout for MaskedLayout {
    fn cache_weight(&self) -> Option<usize> {
        self.layout.cache_weight().map(|weight| {
            weight
                .saturating_add(std::mem::size_of::<Self>())
                .saturating_add(std::mem::size_of_val(self.source_offsets.as_ref()))
                .saturating_add(2 * std::mem::size_of::<usize>())
        })
    }

    fn size(&self) -> (f32, f32) {
        self.layout.size()
    }

    fn hit_test(&self, x: f32, y: f32) -> usize {
        source_offset(&self.source_offsets, self.layout.hit_test(x, y))
    }

    fn hit_position(&self, x: f32, y: f32) -> TextPosition {
        self.to_source(self.layout.hit_position(x, y))
    }

    fn caret(&self, byte_offset: usize) -> Rect {
        self.layout
            .caret(display_offset(&self.source_offsets, byte_offset))
    }

    fn caret_position(&self, position: TextPosition) -> Rect {
        self.layout.caret_position(self.to_display(position))
    }

    fn selection(&self, range: Range<usize>) -> Vec<Rect> {
        if range.start == range.end {
            return Vec::new();
        }
        let start = range.start.min(range.end);
        let end = range.start.max(range.end);
        // Preedit can join an adjacent grapheme. Include the whole intersected cell.
        let end = self
            .source_offsets
            .partition_point(|offset| *offset < end)
            .min(self.source_offsets.len() - 1)
            * BULLET.len();
        self.layout
            .selection(display_offset(&self.source_offsets, start)..end)
    }

    fn visual_line_range(&self, position: TextPosition) -> (TextPosition, TextPosition) {
        let (start, end) = self.layout.visual_line_range(self.to_display(position));
        (self.to_source(start), self.to_source(end))
    }

    fn visual_line_edge(&self, position: TextPosition, end: bool) -> TextPosition {
        self.to_source(self.layout.visual_line_edge(self.to_display(position), end))
    }
}
