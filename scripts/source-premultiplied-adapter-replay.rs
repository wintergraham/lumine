//! CPU-only adapter replay; Draw/Quad mirror only fields consumed by the real
//! paint helpers. This is not a GPU library ABI or policy-qualification test.
#![allow(dead_code)]
use lumine::{
    affine::Affine,
    scene::{Color, NodeId, Rect},
};
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    fade: [f32; 4],
    options: [f32; 4],
    shape: [f32; 4],
    border: [f32; 4],
    /// A fade mask with axis bounds and signed edge bands from a
    /// `fade_edges` ancestor; zero bands leave the quad as is.
    mask: [f32; 4],
}
const NO_MASK: [f32; 4] = [0.; 4];
struct Draw {
    transform: Affine,
    blur: Option<()>,
    clip: Rect,
    bounds: Rect,
    layer: Option<NodeId>,
    image: Option<u64>,
    start: u32,
    end: u32,
}
#[path = "../crates/lumine-gpu/src/source_fill.rs"]
mod source_fill;
#[path = "../crates/lumine-gpu/src/source_fill_paint.rs"]
mod source_fill_paint;
#[path = "../crates/lumine-gpu/src/source_skia154.rs"]
mod source_skia154;
fn rgba(c: Color, opacity: f32) -> [f32; 4] {
    [
        c.0 as f32 / 255.,
        c.1 as f32 / 255.,
        c.2 as f32 / 255.,
        c.3 as f32 / 255. * opacity,
    ]
}
fn rect_outside(rect: Rect, dest: Rect) -> Vec<Rect> {
    [
        Rect::new(rect.x, rect.y, rect.width, dest.y - rect.y),
        Rect::new(
            rect.x,
            dest.y + dest.height,
            rect.width,
            rect.y + rect.height - dest.y - dest.height,
        ),
        Rect::new(rect.x, dest.y, dest.x - rect.x, dest.height),
        Rect::new(
            dest.x + dest.width,
            dest.y,
            rect.x + rect.width - dest.x - dest.width,
            dest.height,
        ),
    ]
    .into_iter()
    .filter(|r| r.width > 0. && r.height > 0.)
    .collect()
}

#[path = "../crates/lumine-gpu/src/source_rgba_paint.rs"]
mod source_rgba_paint;
fn reject_unknown_prior_paint() {
    use lumine::scene::*;
    let mut scene = Scene::new(96., 80.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let node = scene.append(
        scene.root(),
        NodeKind::Quad(QuadStyle {
            fill: Color(169, 190, 230, 20),
            radius: 4.,
            ..Default::default()
        }),
        Style {
            width: Some(48.),
            height: Some(17.),
            ..Default::default()
        },
    );
    scene.set_transform(node, Transform { x: 8., y: 8. });
    scene.flush();
    let item = scene.paint_items().find(|item| item.id == node).unwrap();
    let viewport = Rect::new(0., 0., 96., 80.);
    let native = Quad {
        rect: [20., 15., 6., 6.],
        uv: [0.; 4],
        color: rgba(Color(255, 0, 0, 91), 1.),
        fade: [0.; 4],
        options: [0.; 4],
        shape: [0.; 4],
        border: [0.; 4],
        mask: NO_MASK,
    };
    let source = Quad {
        rect: [8., 8., 48., 17.],
        uv: [0.; 4],
        color: rgba(Color(169, 190, 230, 20), 1.),
        fade: [8., 8., 48., 17.],
        options: [0.; 4],
        shape: [4., 0., 0., 1.],
        border: [0.; 4],
        mask: NO_MASK,
    };
    for mode in 0..4 {
        let mut quads = vec![native, source];
        let before = quads.clone();
        let mut known = rustc_hash::FxHashMap::default();
        let mut opaque = rustc_hash::FxHashMap::default();
        if mode != 0 {
            opaque.insert(
                0,
                source_fill_paint::OpaqueCore {
                    rect: Rect::new(20., 15., 6., 6.),
                    rgb: [255, 0, 0],
                },
            );
        }
        let draw = Draw {
            transform: if mode == 3 {
                Affine {
                    a: 2.,
                    d: 2.,
                    ..Affine::IDENTITY
                }
            } else {
                Affine::IDENTITY
            },
            blur: None,
            clip: viewport,
            bounds: viewport,
            layer: (mode == 1).then_some(scene.root()),
            image: (mode == 2).then_some(1),
            start: 0,
            end: 1,
        };
        assert!(
            source_rgba_paint::resolve(
                &mut quads,
                1,
                &[draw],
                &mut known,
                &opaque,
                &item,
                viewport,
                2.,
                Color(0, 0, 0, 0),
                viewport
            )
            .is_none()
        );
        assert_eq!(
            quads, before,
            "unknown native transaction must remain unchanged"
        );
        assert!(known.is_empty());
    }
}
fn main() {
    reject_unknown_prior_paint();
    use lumine::scene::*;
    let f: Vec<f32> = std::env::args()
        .skip(1)
        .map(|v| v.parse().unwrap())
        .collect();
    let background = Color(f[0] as u8, f[1] as u8, f[2] as u8, f[3] as u8);
    let count = f[4] as usize;
    assert_eq!(f.len(), 5 + count * 12);
    let mut scene = Scene::new(96., 80.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    let mut ids = Vec::new();
    for i in 0..count {
        let n = &f[5 + i * 12..5 + (i + 1) * 12];
        let parent = if n[0] < 0. {
            scene.root()
        } else {
            ids[n[0] as usize]
        };
        let quad = QuadStyle {
            fill: Color(n[6] as u8, n[7] as u8, n[8] as u8, n[9] as u8),
            radius: n[5],
            ..Default::default()
        };
        let has_children = (i + 1..count).any(|j| f[5 + j * 12] == i as f32);
        let kind = if has_children {
            NodeKind::Panel {
                layout: Layout::Overlay,
                quad,
            }
        } else {
            NodeKind::Quad(quad)
        };
        let id = scene.append(
            parent,
            kind,
            Style {
                width: Some(n[3]),
                height: Some(n[4]),
                clip: n[10] > 0.,
                ..Default::default()
            },
        );
        scene.set_transform(id, Transform { x: n[1], y: n[2] });
        if n[11] != 1. {
            scene.set_effects(
                id,
                Effects {
                    opacity: n[11],
                    ..Default::default()
                },
            );
        }
        ids.push(id);
    }
    scene.flush();
    let mut quads = Vec::new();
    let mut draws = Vec::new();
    let mut opaque = rustc_hash::FxHashMap::default();
    let mut known = rustc_hash::FxHashMap::default();
    let viewport = Rect::new(0., 0., 96., 80.);
    let initial = if background.3 == 0 {
        [0; 4]
    } else {
        [background.0, background.1, background.2, background.3]
    };
    let mut image = vec![initial; 192 * 160];
    for item in scene.paint_items() {
        let (color, radius) = match item.kind {
            NodeKind::Quad(q) | NodeKind::Panel { quad: q, .. } => (q.fill, q.radius),
            _ => continue,
        };
        if color.3 == 0 && item.effects.opacity == 1. && !item.isolated {
            continue;
        }
        let start = quads.len() as u32;
        let r = [
            item.bounds.x,
            item.bounds.y,
            item.bounds.width,
            item.bounds.height,
        ];
        let q = Quad {
            rect: r,
            fade: r,
            color: rgba(color, item.effects.opacity),
            shape: [radius, 0., 0., 1.],
            options: [0.; 4],
            border: [0.; 4],
            uv: [0.; 4],
            mask: NO_MASK,
        };
        quads.push(q);
        let clip = item.clip.unwrap_or(viewport);
        if source_rgba_paint::resolve(
            &mut quads, start, &draws, &mut known, &opaque, &item, clip, 2., background, viewport,
        )
        .is_none()
        {
            std::process::exit(3);
        }
        source_fill_paint::remember_opaque(&quads, start, &item, clip, 2., &mut opaque);
        for q in &quads[start as usize..] {
            assert!(source_rgba_paint::resolved(q));
            let [x, y, w, h] = q.rect.map(|v| (v * 2.).round() as usize);
            let color = [
                (q.color[0] * q.color[3] * 255.).round() as u8,
                (q.color[1] * q.color[3] * 255.).round() as u8,
                (q.color[2] * q.color[3] * 255.).round() as u8,
                (q.color[3] * 255.).round() as u8,
            ];
            for yy in y..y + h {
                for xx in x..x + w {
                    image[yy * 192 + xx] = color;
                }
            }
        }
        draws.push(Draw {
            transform: Affine::IDENTITY,
            blur: None,
            clip,
            bounds: viewport,
            layer: None,
            image: None,
            start,
            end: quads.len() as u32,
        });
    }
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    for pixel in image {
        out.write_all(&pixel).unwrap();
    }
}
