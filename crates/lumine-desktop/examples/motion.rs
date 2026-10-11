//! Native motion playground. Inspector snapshots never request frames or poll.
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};
use lumine::{
    compose::prelude::*,
    image::{ShaderInstance, ShaderUniforms},
    input::InputEvent,
};
use lumine_desktop::{Application, WindowOptions};

const SIZE: [u32; 2] = [480, 180];
const BAYER: [f32; 16] = [
    0., 8., 2., 10., 12., 4., 14., 6., 3., 11., 1., 9., 15., 7., 13., 5.,
];
const SHADER: &str = r#"
@group(0) @binding(0) var<storage,read> p:array<f32>;
@group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>;
const bayer=array<f32,16>(0.,8.,2.,10.,12.,4.,14.,6.,3.,11.,1.,9.,15.,7.,13.,5.);
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
 let size=textureDimensions(out); if any(id.xy>=size) {return;}
 let uv=vec2<f32>(id.xy)/vec2<f32>(size);
 let field=clamp(1.-length(uv-vec2(.5))*1.3,0.,1.);
 let cell=max(1u,u32(p[1]));
 let threshold=bayer[((id.y/cell)%4u)*4u+(id.x/cell)%4u]/16.;
 let level=mix(field,floor(field*4.+threshold)/4.,p[0]);
 textureStore(out,vec2<i32>(id.xy),vec4(level*.25,level*.6,level,1.));
}"#;
#[derive(Clone, Copy)]
struct DitherUniforms {
    strength: f32,
    cell: f32,
}
impl ShaderUniforms for DitherUniforms {
    fn encode(&self) -> Vec<f32> {
        vec![self.strength, self.cell]
    }
}
impl DitherUniforms {
    // Lazy software fallback matches WGSL; GPU rendering never builds these pixels.
    fn pixels(self) -> Arc<[u8]> {
        let mut pixels = Vec::with_capacity((SIZE[0] * SIZE[1] * 4) as usize);
        let cell = (self.cell as u32).max(1);
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let dx = x as f32 / SIZE[0] as f32 - 0.5;
                let dy = y as f32 / SIZE[1] as f32 - 0.5;
                let field = (1. - dx.hypot(dy) * 1.3).clamp(0., 1.);
                let threshold = BAYER[(((y / cell) % 4) * 4 + (x / cell) % 4) as usize] / 16.;
                let quantized = (field * 4. + threshold).floor() / 4.;
                let level = field + (quantized - field) * self.strength;
                pixels.extend([level * 0.25, level * 0.6, level].map(|v| (v * 255.).round() as u8));
                pixels.push(255);
            }
        }
        pixels.into()
    }
}
fn control(label: &str, click: impl FnMut() + 'static) -> View {
    button()
        .id(label)
        .px(12.)
        .py(8.)
        .rounded(6.)
        .bg(rgb(0x344159))
        .child(text(label))
        .on_click(click)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "lumine motion playground".into(),
            width: 900.,
            height: 900.,
            ..Default::default()
        })
        .run(|window| {
            window.render(playground());
        })
}
fn playground() -> View {
    component(|cx| {
        let timeline = Timeline::new(cx);
        let duration = Duration::from_secs(3);
        let easing = Easing::cubic_bezier(0.16, 1., 0.3, 1.);
        let pose = timeline
            .track(
                Duration::ZERO,
                duration,
                Keyframes::new([
                    Keyframe::new(0., Vec2::default()).easing(easing),
                    Keyframe::new(0.5, Vec2::new(120., -8.)).easing(easing),
                    Keyframe::new(1., Vec2::default()),
                ])
                .unwrap(),
            )
            .unwrap();
        let opacity = timeline
            .track(
                Duration::ZERO,
                duration,
                Keyframes::new([
                    Keyframe::new(0., 0.65),
                    Keyframe::new(0.5, 1.),
                    Keyframe::new(1., 0.65),
                ])
                .unwrap(),
            )
            .unwrap();
        let blur = timeline
            .track(
                Duration::ZERO,
                duration,
                Keyframes::new([
                    Keyframe::new(0., 0.),
                    Keyframe::new(0.5, 12.),
                    Keyframe::new(1., 0.),
                ])
                .unwrap(),
            )
            .unwrap();
        let dither = timeline
            .track(
                Duration::ZERO,
                duration,
                Keyframes::new([
                    Keyframe::new(0., 0.),
                    Keyframe::new(0.5, 1.),
                    Keyframe::new(1., 0.),
                ])
                .unwrap(),
            )
            .unwrap();
        let presence = Presence::new(cx, true);
        let rotation = timeline
            .track(
                Duration::ZERO,
                duration,
                Keyframes::new([
                    Keyframe::new(0., 0.),
                    Keyframe::new(0.5, 0.08),
                    Keyframe::new(1., 0.),
                ])
                .unwrap(),
            )
            .unwrap();
        let angle = cx.state(0.);
        let manual_angle = angle.clone();
        let expanded = cx.state(false);
        let expansion = expanded.clone();
        let shared = SharedLayoutScope::new(cx);
        presence.progress.label("presence / opacity");
        let exit = MotionPoint::new(cx, Vec2::default());
        exit.x.label("exit / x");
        exit.y.label("exit / y");
        let instance = Rc::new(RefCell::new(ShaderInstance::new(SHADER)));
        let mounted = presence.mounted();
        let presence_opacity = presence.progress.signal();
        let exit_pose = exit.signal();
        let card = provide(
            shared,
            switch(
                move || (mounted.get(), expansion.get()),
                move |(mounted, expanded), _| {
                    if !mounted {
                        return div().hidden();
                    }
                    let (pose, opacity, presence_opacity, exit_pose) = (
                        pose.clone(),
                        opacity.clone(),
                        presence_opacity.clone(),
                        exit_pose.clone(),
                    );
                    let (dither, instance, blur) = (dither.clone(), instance.clone(), blur.clone());
                    let (rotation, angle) = (rotation.clone(), manual_angle.clone());
                    overlay()
                        .id("motion-card")
                        .layout_id("dither-card")
                        .layout_motion(Transition::spring(Spring::default()))
                        .size(
                            if expanded { 640. } else { 480. },
                            if expanded { 240. } else { 180. },
                        )
                        .child(
                            image_signal("Persistent dither field", move || {
                                let uniforms = DitherUniforms {
                                    strength: dither.get(),
                                    cell: 2.,
                                };
                                instance
                                    .borrow_mut()
                                    .render(
                                        SIZE[0],
                                        SIZE[1],
                                        &uniforms,
                                        [SIZE[0].div_ceil(8), SIZE[1].div_ceil(8)],
                                        move || uniforms.pixels(),
                                    )
                                    .unwrap()
                            })
                            .w_full()
                            .h_full(),
                        )
                        .child(
                            div()
                                .size(300., 94.)
                                .p(18.)
                                .rounded(12.)
                                .bg(rgba(0x10182473))
                                .child(text("One transport, five properties").text_size(18.))
                                .child(text("Translation / rotation / opacity / blur / dither"))
                                .reactive_style(move || {
                                    Styles::new().translate(24., 44.).blur(blur.get())
                                }),
                        )
                        .reactive_style(move || {
                            let (p, e) = (pose.get(), exit_pose.get());
                            Styles::new()
                                .translate(p.x + e.x, p.y + e.y)
                                .rotate(rotation.get() + angle.get())
                                .opacity(opacity.get() * presence_opacity.get())
                        })
                },
            ),
        );
        let expand = control("Card / expanded panel", move || {
            expanded.update(|value| *value = !*value);
        });
        let rotation_control = slider("Rotation (radians)", angle, -0.3..=0.3).size(300., 36.);
        let mut open = true;
        let toggle = control("Toggle grouped exit", move || {
            open = !open;
            if open {
                exit.animate_to(Vec2::default(), Transition::spring(Spring::default()));
                presence.set_present(true, Transition::tween(Duration::from_millis(180), easing));
            } else {
                let group = AnimationGroup::new([
                    exit.x
                        .animate_to(32., Transition::tween(Duration::from_millis(240), easing)),
                    exit.y
                        .animate_to(24., Transition::tween(Duration::from_millis(450), easing)),
                ]);
                presence.set_present_with(
                    false,
                    Transition::tween(Duration::from_millis(180), easing),
                    group,
                );
            }
        });
        let progress = timeline.progress();
        let status = text_signal(move || {
            format!(
                "Timeline {:>3.0}% — close/reopen during exit to interrupt it",
                progress.get() * 100.
            )
        });
        let transport = row()
            .gap(8.)
            .child(control("Play / resume", {
                let t = timeline.clone();
                move || {
                    t.play();
                }
            }))
            .child(control("Pause", {
                let t = timeline.clone();
                move || t.pause()
            }))
            .child(control("Restart", {
                let t = timeline.clone();
                move || {
                    t.restart();
                }
            }))
            .child(control("Seek 50%", {
                let t = timeline.clone();
                move || t.seek(Duration::from_millis(1500))
            }))
            .child(control("0.5×", {
                let t = timeline.clone();
                move || t.set_rate(0.5)
            }))
            .child(control("1×", {
                let t = timeline.clone();
                move || t.set_rate(1.)
            }))
            .child(control("2×", move || timeline.set_rate(2.)));
        let drag = DragMotion::new(cx, 0., MotionAxis::X, 0.0..=360.0)
            .unwrap()
            .snap_points([0., 180., 360.])
            .unwrap();
        let drag_position = drag.signal();
        let states = MotionStates::new(
            cx,
            "rest",
            [
                ("rest", 0., Transition::spring(Spring::default())),
                ("hover", 1., Transition::spring(Spring::default())),
            ],
        )
        .unwrap();
        states.value.label("named hover state");
        let hover = states.signal();
        let draggable = drag.bind(
            div()
                .size(128., 48.)
                .p(12.)
                .rounded(8.)
                .bg(rgb(0x718aff))
                .child(text("Drag / snap"))
                .reactive_style(move || {
                    let hover = hover.get();
                    Styles::new()
                        .translate(drag_position.get(), -3. * hover)
                        .scale(1. + 0.06 * hover, 1. + 0.06 * hover)
                })
                .on_event(move |event| match event.event {
                    InputEvent::PointerEnter => {
                        states.set("hover").unwrap();
                    }
                    InputEvent::PointerLeave => {
                        states.set("rest").unwrap();
                    }
                    _ => {}
                }),
        );
        // This retained section projects its layout when the card unmounts.
        let dragging = column().gap(8.).layout_motion(Transition::spring(Spring::default()))
                .child(text("Drag to 0 / 180 / 360. Release velocity selects the snap; named rest/hover states."))
                .child(div().size(500., 54.).bg(rgb(0x202c40)).child(draggable));
        let scroll_progress = ScrollProgress::new(cx);
        let fraction = scroll_progress.signal();
        let scrolling = scroll_progress.bind(
            scroll(cx.state(0.))
                .size(360., 120.)
                .scrollbar(true)
                .children((0..10_u32).map(|i| {
                    div()
                        .h(32.)
                        .p(6.)
                        .bg(rgb(if i.is_multiple_of(2) {
                            0x202c40
                        } else {
                            0x28364c
                        }))
                        .child(text(format!("Scroll row {}", i + 1)))
                })),
        );
        let indicator =
            text_signal(move || format!("ScrollProgress: {:.0}%", fraction.get() * 100.));
        let inspector = MotionInspector::new(cx);
        let report = cx.state(String::from("Inspector snapshots only when requested."));
        let snapshot = control("Snapshot inspector", {
            let report = report.clone();
            move || {
                let snapshot = inspector.snapshot();
                let mut lines = format!(
                    "{} active / {} tracks; frame requested: {}; sample: {:.1} µs / {} tracks",
                    snapshot.active_tracks,
                    snapshot.total_tracks,
                    snapshot.display_frame_requested,
                    snapshot.last_sample_cost.as_secs_f64() * 1e6,
                    snapshot.last_sampled_tracks
                );
                if let Some(r) = snapshot.renderer {
                    lines.push_str(&format!(
                        "\nFrame {}: layout {}; shader dispatch {}; allocations {}; blur passes {}",
                        r.frame,
                        r.layout_nodes,
                        r.shader_dispatches,
                        r.shader_allocations,
                        r.blur_passes
                    ));
                }
                for track in snapshot.tracks.iter().take(4) {
                    lines.push_str(&format!(
                        "\n{}: {:?}, value {:.2}, velocity {:.2}, rate {:.1}×",
                        track.label, track.state, track.value, track.velocity, track.rate
                    ));
                }
                report.set(lines);
            }
        });
        scroll(cx.state(0.))
            .w_full()
            .h_full()
            .scrollbar(true)
            .bg(rgb(0x141c2a))
            .child(
                column()
                    .p(24.)
                    .gap(14.)
                    .w_full()
                    .bg(rgb(0x141c2a))
                    .text_color(rgb(0xe8eef8))
                    .child(text("Motion playground").text_size(26.))
                    .child(transport)
                    .child(status)
                    .child(
                        row()
                            .gap(12.)
                            .child(toggle)
                            .child(expand)
                            .child(rotation_control),
                    )
                    .child(card)
                    .child(dragging)
                    .child(
                        row()
                            .gap(20.)
                            .child(scrolling)
                            .child(column().gap(10.).child(indicator).child(snapshot)),
                    )
                    .child(text_signal(move || report.get()).text_size(12.)),
            )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_dither_shader_matches_fallback_and_reuses_uniform_resources() {
        use lumine::scene::{Layout, NodeKind, Scene, Style};
        use lumine_gpu::GpuRenderer;
        let mut instance = ShaderInstance::new(SHADER);
        let mut scene = Scene::new(SIZE[0] as f32, SIZE[1] as f32);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let mut renderer = GpuRenderer::new(SIZE[0], SIZE[1]).unwrap();
        let mut node = None;
        for strength in [0., 1.] {
            let uniforms = DitherUniforms { strength, cell: 2. };
            let expected = uniforms.pixels();
            let image = instance
                .render(
                    SIZE[0],
                    SIZE[1],
                    &uniforms,
                    [SIZE[0].div_ceil(8), SIZE[1].div_ceil(8)],
                    || panic!("GPU must not evaluate the fallback"),
                )
                .unwrap();
            if let Some(node) = node {
                scene.set_kind(node, NodeKind::Image(image));
            } else {
                node = Some(scene.append(
                    scene.root(),
                    NodeKind::Image(image),
                    Style {
                        width: Some(SIZE[0] as f32),
                        height: Some(SIZE[1] as f32),
                        ..Default::default()
                    },
                ));
            }
            let report = scene.flush();
            let stats = renderer.render(&scene, &report.damage).unwrap();
            assert_eq!(stats.shader_dispatches, 1);
            assert_eq!(
                stats.shader_resource_allocations,
                usize::from(strength == 0.)
            );
            let actual = renderer.readback().unwrap();
            assert!(
                actual
                    .iter()
                    .zip(expected.iter())
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
    }
    #[test]
    fn playground_mounts_idle_without_an_inspector_polling_task() {
        let mut ui = lumine::widgets::Ui::new(900., 900.);
        let frames = lumine::frame::FrameClock::new();
        let root = ui.mount(provide(frames.clone(), playground()));
        ui.prepare_frame();
        assert!(ui.scene.borrow().contains(root.node()));
        assert!(!frames.wants_frame());
        drop(root);
        assert!(!frames.wants_frame());
    }
    #[test]
    fn software_shader_fallback_is_opaque_and_strength_changes_pixels() {
        let smooth = DitherUniforms {
            strength: 0.,
            cell: 2.,
        }
        .pixels();
        let dithered = DitherUniforms {
            strength: 1.,
            cell: 2.,
        }
        .pixels();
        assert_eq!(smooth.len(), (SIZE[0] * SIZE[1] * 4) as usize);
        assert!(
            smooth
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
        assert_ne!(smooth, dithered);
        let center = ((SIZE[1] / 2 * SIZE[0] + SIZE[0] / 2) * 4) as usize;
        assert_eq!(&dithered[center..center + 4], &[64, 153, 255, 255]);
    }
    #[test]
    fn shared_card_remount_and_transport_obey_reduced_motion_without_idle_frames() {
        use lumine::{compose::TaskRunner, motion::MotionPolicy, task::LocalExecutor};
        let mut ui = lumine::widgets::Ui::new(900., 900.);
        let frames = lumine::frame::FrameClock::new();
        let executor = Rc::new(RefCell::new(LocalExecutor::new()));
        let policy = MotionPolicy {
            active: ui.signal(true),
            reduced: ui.signal(true),
        };
        let root = ui.mount(provide(
            frames.clone(),
            provide(
                TaskRunner::from_executor(executor.clone()),
                provide(policy, playground()),
            ),
        ));
        ui.prepare_frame();
        let original = root.find("motion-card").unwrap();
        assert_eq!(ui.scene.borrow().layout_bounds(original).width, 480.);
        let expand = root.find("Card / expanded panel").unwrap();
        ui.input
            .dispatch_to(&ui.scene, expand, InputEvent::Activate);
        executor.borrow_mut().tick();
        ui.prepare_frame();
        let replacement = root.find("motion-card").unwrap();
        assert_ne!(original, replacement);
        assert_eq!(ui.scene.borrow().layout_bounds(replacement).width, 640.);
        assert_eq!(ui.scene.borrow().bounds(replacement).width, 640.);
        let restart = root.find("Restart").unwrap();
        ui.input
            .dispatch_to(&ui.scene, restart, InputEvent::Activate);
        executor.borrow_mut().tick();
        ui.prepare_frame();
        assert!(!frames.wants_frame());
        root.unmount();
        executor.borrow_mut().tick();
        assert!(!frames.wants_frame());
    }
}
