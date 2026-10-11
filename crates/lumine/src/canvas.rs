//! Retained, backend-independent vector drawing in local logical coordinates.
use crate::{
    affine::Affine,
    scene::{Color, Rect},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Move(Point),
    Line(Point),
    Quadratic(Point, Point),
    Cubic(Point, Point, Point),
    Close,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    segments: Arc<[Segment]>,
}
impl Path {
    pub fn builder() -> PathBuilder {
        PathBuilder::default()
    }
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    pub fn rectangle(rect: Rect) -> Self {
        let mut path = Self::builder();
        path.move_to(rect.x, rect.y)
            .line_to(rect.x + rect.width, rect.y)
            .line_to(rect.x + rect.width, rect.y + rect.height)
            .line_to(rect.x, rect.y + rect.height)
            .close();
        path.build().expect("finite rectangle")
    }
}
#[derive(Default)]
pub struct PathBuilder {
    segments: Vec<Segment>,
    transform: Affine,
}
impl PathBuilder {
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.segments.push(Segment::Move(Point::new(x, y)));
        self
    }
    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.segments.push(Segment::Line(Point::new(x, y)));
        self
    }
    pub fn quadratic_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.segments
            .push(Segment::Quadratic(Point::new(cx, cy), Point::new(x, y)));
        self
    }
    pub fn cubic_to(&mut self, c1: Point, c2: Point, end: Point) -> &mut Self {
        self.segments.push(Segment::Cubic(c1, c2, end));
        self
    }
    pub fn append(&mut self, path: &Path) -> &mut Self {
        self.segments.extend_from_slice(path.segments());
        self
    }
    pub fn close(&mut self) -> &mut Self {
        self.segments.push(Segment::Close);
        self
    }
    /// Append a circular arc, connecting to its start with a line. Angles are
    /// clockwise radians; sweeps are bounded to one full turn in either direction.
    pub fn arc(&mut self, center: Point, radius: f32, start: f32, sweep: f32) -> &mut Self {
        if ![center.x, center.y, radius, start, sweep]
            .iter()
            .all(|v| v.is_finite())
            || radius < 0.
        {
            return self.line_to(f32::NAN, f32::NAN);
        }
        let sweep = sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
        let steps = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.) as usize;
        let delta = sweep / steps as f32;
        let first = Point::new(
            center.x + radius * start.cos(),
            center.y + radius * start.sin(),
        );
        if self.segments.is_empty() {
            self.move_to(first.x, first.y);
        } else {
            self.line_to(first.x, first.y);
        }
        for index in 0..steps {
            let a = start + index as f32 * delta;
            let b = a + delta;
            let k = 4. / 3. * (delta / 4.).tan();
            self.cubic_to(
                Point::new(
                    center.x + radius * (a.cos() - k * a.sin()),
                    center.y + radius * (a.sin() + k * a.cos()),
                ),
                Point::new(
                    center.x + radius * (b.cos() + k * b.sin()),
                    center.y + radius * (b.sin() - k * b.cos()),
                ),
                Point::new(center.x + radius * b.cos(), center.y + radius * b.sin()),
            );
        }
        self
    }
    pub fn transform(&mut self, transform: Affine) -> &mut Self {
        self.transform = self.transform.then(transform);
        self
    }
    pub fn build(self) -> Result<Path, &'static str> {
        if self.segments.len() > 65536 {
            return Err("path exceeds 65536 segments");
        }
        let transform = self.transform;
        let point = |p: Point| -> Result<Point, &'static str> {
            let (x, y) = transform.point(p.x, p.y);
            if x.is_finite() && y.is_finite() {
                Ok(Point::new(x, y))
            } else {
                Err("path coordinates must be finite")
            }
        };
        let segments = self
            .segments
            .into_iter()
            .map(|s| {
                Ok(match s {
                    Segment::Move(p) => Segment::Move(point(p)?),
                    Segment::Line(p) => Segment::Line(point(p)?),
                    Segment::Quadratic(a, b) => Segment::Quadratic(point(a)?, point(b)?),
                    Segment::Cubic(a, b, c) => Segment::Cubic(point(a)?, point(b)?, point(c)?),
                    Segment::Close => Segment::Close,
                })
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        Ok(Path {
            segments: segments.into(),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    Solid(Color),
    Linear {
        start: Point,
        end: Point,
        stops: Arc<[GradientStop]>,
    },
    /// Diagonal parallel stripes in logical coordinates.
    Slash {
        background: Color,
        foreground: Color,
        spacing: f32,
        width: f32,
    },
}
impl From<Color> for Brush {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}
impl Brush {
    pub fn linear(
        start: Point,
        end: Point,
        stops: impl Into<Arc<[GradientStop]>>,
    ) -> Result<Self, &'static str> {
        let stops = stops.into();
        if ![start.x, start.y, end.x, end.y]
            .iter()
            .all(|v| v.is_finite())
            || start == end
            || stops.len() < 2
            || stops.len() > 1024
            || stops
                .iter()
                .any(|s| !s.offset.is_finite() || !(0. ..=1.).contains(&s.offset))
            || stops.windows(2).any(|s| s[0].offset > s[1].offset)
        {
            return Err("invalid linear gradient");
        }
        Ok(Self::Linear { start, end, stops })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    Winding,
    EvenOdd,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dash: Vec<f32>,
    pub dash_offset: f32,
}
impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.,
            dash: vec![],
            dash_offset: 0.,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Clip {
        path: Path,
        rule: FillRule,
    },
    Restore,
    Fill {
        path: Path,
        brush: Brush,
        rule: FillRule,
    },
    Stroke {
        path: Path,
        brush: Brush,
        style: Stroke,
    },
}
/// Equal command lists preserve pixels, uploads and retained layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    commands: Vec<Draw>,
}
impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn fill(&mut self, path: Path, brush: impl Into<Brush>) -> &mut Self {
        self.fill_rule(path, brush, FillRule::Winding)
    }
    pub fn fill_rule(&mut self, path: Path, brush: impl Into<Brush>, rule: FillRule) -> &mut Self {
        self.commands.push(Draw::Fill {
            path,
            brush: brush.into(),
            rule,
        });
        self
    }
    pub fn stroke(&mut self, path: Path, brush: impl Into<Brush>, style: Stroke) -> &mut Self {
        self.commands.push(Draw::Stroke {
            path,
            brush: brush.into(),
            style,
        });
        self
    }
    pub fn clip(&mut self, path: Path, rule: FillRule) -> &mut Self {
        self.commands.push(Draw::Clip { path, rule });
        self
    }
    pub fn restore(&mut self) -> &mut Self {
        self.commands.push(Draw::Restore);
        self
    }
    pub fn commands(&self) -> &[Draw] {
        &self.commands
    }
}
