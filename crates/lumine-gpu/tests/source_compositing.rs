//! Shared, opt-in CPU-source constant fills. Expectations come from actual
//! independent Linux Chromium154 software readbacks, never this GPU renderer.
//! AA rims and unproven/layered paint remain strict and retain existing pixels.
use lumine::{affine::Affine, scene::*};
use lumine_gpu::{GpuRenderer, SourceFillCompositing};

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
fn fixed(w: f32, h: f32) -> Style {
    Style {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    }
}
fn scene() -> Scene {
    let mut scene = Scene::new(96., 80.);
    scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
    scene
}
fn fill(scene: &mut Scene, parent: NodeId, color: Color) -> NodeId {
    let n = scene.append(
        parent,
        NodeKind::Quad(QuadStyle {
            fill: color,
            radius: 4.,
            ..Default::default()
        }),
        fixed(48., 17.),
    );
    scene.set_transform(n, Transform { x: 8., y: 8. });
    n
}
fn gpu() -> GpuRenderer {
    let mut gpu = GpuRenderer::new(192, 160).unwrap();
    gpu.set_scale_factor(2.);
    gpu.set_background(Color(26, 26, 24, 255));
    gpu
}
fn pixels(gpu: &mut GpuRenderer, scene: &mut Scene, enabled: bool) -> Vec<u8> {
    gpu.set_source_fill_compositing(if enabled {
        SourceFillCompositing::SkiaSoftwareSrgb8OpaqueBackdrop
    } else {
        SourceFillCompositing::PreserveExisting
    });
    let report = scene.flush();
    gpu.render(scene, &report.damage).unwrap();
    gpu.readback().unwrap()
}
fn pixel(bytes: &[u8], x: usize, y: usize) -> &[u8] {
    &bytes[(y * 192 + x) * 4..(y * 192 + x + 1) * 4]
}

#[test]
fn three_captured_source_fills_exact_interior_and_unchanged_rims() {
    let _serial = serial();
    let mut gpu = gpu();
    for (color, expected) in [
        (Color(169, 190, 230, 20), [36, 38, 40, 255]),
        (Color(187, 176, 242, 20), [38, 37, 41, 255]),
        (Color(166, 212, 174, 20), [36, 40, 36, 255]),
    ] {
        let mut scene = scene();
        let root = scene.root();
        fill(&mut scene, root, color);
        let ordinary = pixels(&mut gpu, &mut scene, false);
        let corrected = pixels(&mut gpu, &mut scene, true);
        assert_eq!(pixel(&corrected, 48, 32), expected);
        // Original geometry: [8,8,48,17], rounded4, DPR2. All contour
        // pixels (and all unrelated pixels) retain exact original bytes.
        for y in 0..160 {
            for x in 0..192 {
                if x < 24 || x >= 104 || y < 24 || y >= 42 {
                    assert_eq!(
                        pixel(&corrected, x, y),
                        pixel(&ordinary, x, y),
                        "rim ({x},{y})"
                    );
                }
            }
        }
        assert_eq!(
            pixels(&mut gpu, &mut scene, false),
            ordinary,
            "disable restores retained pixels"
        );
    }
}

#[test]
fn actual_order_selects_last_complete_opaque_backdrop() {
    let _serial = serial();
    let mut gpu = gpu();
    let mut scene = scene();
    let root = scene.root();
    scene.append(
        root,
        NodeKind::Rect(Color(255, 255, 255, 255)),
        fixed(96., 80.),
    );
    scene.append(
        root,
        NodeKind::Rect(Color(64, 128, 192, 255)),
        fixed(96., 80.),
    );
    fill(&mut scene, root, Color(169, 190, 230, 20));
    let result = pixels(&mut gpu, &mut scene, true);
    // Actual source Canvas sweep: foreground A9BEE6 alpha20 over4080C0.
    assert_eq!(pixel(&result, 48, 32), [72, 133, 195, 255]);
}

#[test]
fn intervening_partial_or_translucent_draw_blocks_uniform_backdrop() {
    let _serial = serial();
    let mut gpu = gpu();
    for color in [Color(255, 0, 0, 255), Color(255, 0, 0, 91)] {
        let mut scene = scene();
        let root = scene.root();
        let blocker = scene.append(root, NodeKind::Rect(color), fixed(6., 6.));
        scene.set_transform(blocker, Transform { x: 20., y: 15. });
        fill(&mut scene, root, Color(169, 190, 230, 20));
        let ordinary = pixels(&mut gpu, &mut scene, false);
        assert_eq!(pixels(&mut gpu, &mut scene, true), ordinary);
    }
}

#[test]
fn rounded_ancestor_supports_only_proven_full_coverage() {
    let _serial = serial();
    let mut gpu = gpu();
    let mut scene = scene();
    let root = scene.root();
    let parent = scene.append(
        root,
        NodeKind::Quad(QuadStyle {
            fill: Color(26, 26, 24, 255),
            radius: 4.,
            ..Default::default()
        }),
        Style {
            clip: true,
            ..fixed(96., 80.)
        },
    );
    fill(&mut scene, parent, Color(169, 190, 230, 20));
    assert_eq!(
        pixel(&pixels(&mut gpu, &mut scene, true), 48, 32),
        [36, 38, 40, 255]
    );
}

#[test]
fn opacity_rotation_layer_and_transparent_backdrops_preserve_existing_paint() {
    let _serial = serial();
    let mut gpu = gpu();
    for case in 0..4 {
        let mut scene = scene();
        let root = scene.root();
        let node = fill(&mut scene, root, Color(169, 190, 230, 20));
        gpu.set_background(if case == 3 {
            Color(26, 26, 24, 0)
        } else {
            Color(26, 26, 24, 255)
        });
        match case {
            0 => scene.set_effects(
                node,
                Effects {
                    opacity: 0.5,
                    ..Default::default()
                },
            ),
            1 => scene.set_paint_transform(node, Affine::rotation(0.1)),
            2 => scene.set_isolated(node, true),
            _ => {}
        }
        let ordinary = pixels(&mut gpu, &mut scene, false);
        assert_eq!(
            pixels(&mut gpu, &mut scene, true),
            ordinary,
            "unsupported case{case}"
        );
    }
}
