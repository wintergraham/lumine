//! Shared, opt-in CPU-source constant fills. Expectations come from actual
//! independent Linux Chromium154 software readbacks, never this GPU renderer.
//! Source AA rims remain strict; unsupported/layered paint retains existing pixels.
use lumine::source_paint::{SourcePaintCapability, SourcePaintPolicy, SourcePaintScope};
use lumine::{affine::Affine, scene::*};
use lumine_gpu::{GpuRenderer, SourceFillCompositing};

fn source_scope(owner: u64) -> SourcePaintScope {
    SourcePaintScope::Capability(SourcePaintCapability {
        owner: std::num::NonZeroU64::new(owner).unwrap(),
        contract_sha256: [owner as u8; 32],
        policy: SourcePaintPolicy::SkiaSoftwareSrgb8OpaqueBackdrop,
    })
}

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
fn source_root_scope_preserves_native_chrome_and_retained_toggle() {
    let _serial = serial();
    let mut gpu = gpu();
    let mut scene = scene();
    let root = scene.root();
    let source = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(96., 80.));
    fill(&mut scene, source, Color(169, 190, 230, 20));
    let chrome = fill(&mut scene, root, Color(169, 190, 230, 20));
    scene.set_transform(chrome, Transform { x: 8., y: 40. });
    let error = fill(&mut scene, root, Color(169, 190, 230, 20));
    scene.set_transform(error, Transform { x: 64., y: 8. });
    let ordinary = pixels(&mut gpu, &mut scene, false);
    scene.set_source_paint_scope(source, Some(source_scope(1)));
    let scoped = pixels(&mut gpu, &mut scene, false);
    assert_eq!(pixel(&scoped, 48, 32), [36, 38, 40, 255]);
    for y in 0..160 {
        for x in 0..192 {
            if x >= 112 || y >= 64 {
                assert_eq!(
                    pixel(&scoped, x, y),
                    pixel(&ordinary, x, y),
                    "native ({x},{y})"
                );
            }
        }
    }
    // The explicit renderer-wide diagnostic switch remains available, and is
    // separately observable from the default per-source-root behavior.
    let diagnostic = pixels(&mut gpu, &mut scene, true);
    assert_eq!(pixel(&diagnostic, 48, 96), [36, 38, 40, 255]);
    assert_eq!(pixels(&mut gpu, &mut scene, false), scoped);
    scene.set_source_paint_scope(source, None);
    assert_eq!(pixels(&mut gpu, &mut scene, false), ordinary);
}

#[test]
fn source_root_backdrop_still_observes_unmarked_native_draws() {
    let _serial = serial();
    let mut gpu = gpu();
    for partial in [false, true] {
        let mut scene = scene();
        let root = scene.root();
        let native = scene.append(
            root,
            NodeKind::Rect(Color(64, 128, 192, 255)),
            if partial {
                fixed(6., 6.)
            } else {
                fixed(96., 80.)
            },
        );
        if partial {
            scene.set_transform(native, Transform { x: 20., y: 15. });
        }
        let source = fill(&mut scene, root, Color(169, 190, 230, 20));
        let ordinary = pixels(&mut gpu, &mut scene, false);
        scene.set_source_paint_scope(source, Some(source_scope(1)));
        let scoped = pixels(&mut gpu, &mut scene, false);
        assert_eq!(pixel(&scoped, 48, 32), [72, 133, 195, 255]);
        if partial {
            assert_eq!(pixel(&scoped, 32, 32), [36, 38, 40, 255]);
            assert_eq!(pixel(&scoped, 4, 4), pixel(&ordinary, 4, 4));
        }
    }
}

#[test]
fn nested_source_root_conflicts_and_preserve_barriers_keep_default_pixels() {
    let _serial = serial();
    let mut gpu = gpu();
    for boundary in [source_scope(2), SourcePaintScope::PreserveExisting] {
        let mut scene = scene();
        let root = scene.root();
        let source = scene.append(root, NodeKind::Container(Layout::Overlay), fixed(96., 80.));
        let child = fill(&mut scene, source, Color(169, 190, 230, 20));
        let ordinary = pixels(&mut gpu, &mut scene, false);
        scene.set_source_paint_scope(source, Some(source_scope(1)));
        scene.set_source_paint_scope(child, Some(boundary));
        assert_eq!(pixels(&mut gpu, &mut scene, false), ordinary);
        scene.set_source_paint_scope(child, None);
        assert_eq!(
            pixel(&pixels(&mut gpu, &mut scene, false), 48, 32),
            [36, 38, 40, 255]
        );
    }
}

#[test]
fn three_captured_source_fills_exact_interior_and_source_rim() {
    let _serial = serial();
    let mut gpu = gpu();
    for (color, expected, source_rim) in [
        (
            Color(169, 190, 230, 20),
            [36, 38, 40, 255],
            [27, 28, 27, 255],
        ),
        (
            Color(187, 176, 242, 20),
            [38, 37, 41, 255],
            [28, 28, 27, 255],
        ),
        (
            Color(166, 212, 174, 20),
            [36, 40, 36, 255],
            [27, 28, 26, 255],
        ),
    ] {
        let mut scene = scene();
        let root = scene.root();
        fill(&mut scene, root, color);
        let ordinary = pixels(&mut gpu, &mut scene, false);
        let corrected = pixels(&mut gpu, &mut scene, true);
        assert_eq!(pixel(&corrected, 48, 32), expected);
        // Independently captured original Chromium rim, not native output.
        // Original manifest 1f80c99abbcb9d129d8b03195dc89259c546f7b4327a7e5433c09aeeb5b491f5.
        // The separate strict 20-case differential checks every source pixel.
        assert_eq!(pixel(&corrected, 20, 16), source_rim);
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
fn opaque_partial_backdrops_resolve_per_pixel_and_unknown_translucency_falls_back() {
    let _serial = serial();
    let mut gpu = gpu();
    for color in [Color(255, 0, 0, 255), Color(255, 0, 0, 91)] {
        let mut scene = scene();
        let root = scene.root();
        let blocker = scene.append(root, NodeKind::Rect(color), fixed(6., 6.));
        scene.set_transform(blocker, Transform { x: 20., y: 15. });
        if color.3 < 255 {
            // Keep this earlier paint explicitly unqualified too. Otherwise
            // its own proven Rect core may correctly change before the badge
            // is conservatively rejected for partial backdrop overlap.
            scene.set_effects(
                blocker,
                Effects {
                    opacity: 0.5,
                    ..Default::default()
                },
            );
        }
        fill(&mut scene, root, Color(169, 190, 230, 20));
        let ordinary = pixels(&mut gpu, &mut scene, false);
        let scoped = pixels(&mut gpu, &mut scene, true);
        if color.3 == 255 {
            // Exact original source fixture partial-opaque-backdrop center.
            assert_eq!(pixel(&scoped, 48, 32), [248, 15, 18, 255]);
            assert_eq!(pixel(&scoped, 32, 32), [36, 38, 40, 255]);
        } else {
            assert_eq!(
                scoped, ordinary,
                "unproven translucent draw stays unsupported"
            );
        }
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
        NodeKind::Panel {
            layout: Layout::Overlay,
            quad: QuadStyle {
                fill: Color(26, 26, 24, 255),
                radius: 4.,
                ..Default::default()
            },
        },
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
