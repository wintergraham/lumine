//! Native effects workloads. No readback is included in timed frames.
//! cargo run --release -p lumine-gpu --features benchmark --example effects_bench
//! LUMINE_BENCH_{FRAMES,WARMUP,ONLY,SIZES,SIGMAS,OUTPUT,IN_FLIGHT,TRACE} configure runs.
use std::{collections::BTreeMap, time::Instant};
use lumine::{
    affine::Affine,
    image::{EffectChain, EffectStage, ImageData, ShaderInstance, ShaderUniforms},
    scene::*,
};
use lumine_gpu::{
    BlurAlgorithm, GpuContext, GpuRenderer, GpuStats,
    benchmark::{Config, Trace, emit, summary},
    profiling::GpuFrameProfile,
};

// Animated ordered dither with a moving radial field, rather than a trivial fill.
const DITHER: &str = r#"
@group(0) @binding(0) var<storage,read> p:array<f32>;
@group(0) @binding(1) var out:texture_storage_2d<rgba8unorm,write>;
const bayer=array<f32,16>(0.,8.,2.,10.,12.,4.,14.,6.,3.,11.,1.,9.,15.,7.,13.,5.);
@compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
 let size=textureDimensions(out); if any(id.xy>=size) {return;}
 let uv=vec2<f32>(id.xy)/vec2<f32>(size);
 let phase=p[0]; let cell=max(1u,u32(p[1]));
 let ix=(id.x/cell+u32(phase))%4u; let iy=(id.y/cell)%4u;
 let center=vec2(.5+.25*sin(phase*0.07),.5+.25*cos(phase*0.09));
 let field=clamp(1.-length(uv-center)*1.3,0.,1.);
 let level=floor(field*4.+bayer[iy*4u+ix]/16.)/4.;
 textureStore(out,vec2<i32>(id.xy),vec4(level*0.25,level*0.6,level,1.));
}"#;

struct DitherUniforms {
    phase: f32,
    cell: f32,
}
impl ShaderUniforms for DitherUniforms {
    fn encode(&self) -> Vec<f32> {
        vec![self.phase, self.cell]
    }
}
fn fixed(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}
fn rect(scene: &mut Scene, parent: NodeId, color: Color, area: Rect) -> NodeId {
    let id = scene.append(
        parent,
        NodeKind::Rect(color),
        fixed(area.width, area.height),
    );
    scene.set_transform(
        id,
        Transform {
            x: area.x,
            y: area.y,
        },
    );
    id
}
fn image(
    instance: &mut ShaderInstance,
    width: u32,
    height: u32,
    phase: f32,
) -> std::sync::Arc<lumine::image::ImageData> {
    instance
        .render(
            width,
            height,
            &DitherUniforms { phase, cell: 2. },
            [width.div_ceil(8), height.div_ceil(8)],
            move || vec![0; width as usize * height as usize * 4].into(),
        )
        .unwrap()
}
struct Workload {
    scene: Scene,
    content: NodeId,
    cursor: NodeId,
    panels: Vec<(NodeId, NodeId)>,
    shaders: Vec<(NodeId, ShaderInstance, u32, u32)>,
    chain: Option<(NodeId, EffectChain, std::sync::Arc<ImageData>)>,
}
impl Workload {
    fn new(width: u32, height: u32, mode: &str, sigma: f32) -> Self {
        let (w, h) = (width as f32, height as f32);
        let mut scene = Scene::new(w, h);
        let root = scene.root();
        scene.set_kind(root, NodeKind::Container(Layout::Overlay));
        rect(
            &mut scene,
            root,
            Color(18, 26, 42, 255),
            Rect::new(0., 0., w, h),
        );
        let clip = scene.append(
            root,
            NodeKind::Container(Layout::Overlay),
            Style {
                clip: true,
                ..fixed(w, h)
            },
        );
        let content = scene.append(clip, NodeKind::Container(Layout::Overlay), fixed(w, h * 2.));
        scene.set_scroll_copy(content, true);
        for y in 0..((h * 2. / 24.).ceil() as usize) {
            let row = rect(
                &mut scene,
                content,
                if y % 2 == 0 {
                    Color(28, 45, 67, 255)
                } else {
                    Color(42, 32, 58, 255)
                },
                Rect::new(0., y as f32 * 24., w, 24.),
            );
            for x in 0..12 {
                rect(
                    &mut scene,
                    row,
                    Color((x * 17) as u8, (y * 13 % 255) as u8, 150, 255),
                    Rect::new(x as f32 * w / 12. + 5., 4., w / 12. - 10., 16.),
                );
            }
        }
        let blur = matches!(
            mode,
            "overlap" | "scroll" | "foreground" | "resize" | "transition"
        );
        let mut panels = Vec::new();
        let mut shaders = Vec::new();
        if blur {
            for n in 0..4 {
                let panel = scene.append(
                    root,
                    NodeKind::Container(Layout::Overlay),
                    fixed(w * 0.38, h * 0.48),
                );
                scene.set_transform(
                    panel,
                    Transform {
                        x: w * (0.09 + n as f32 * 0.14),
                        y: h * (0.08 + n as f32 * 0.13),
                    },
                );
                scene.set_effects(
                    panel,
                    Effects {
                        blur_radius: sigma,
                        edge_fade: 4.,
                        ..Default::default()
                    },
                );
                let fill = rect(
                    &mut scene,
                    panel,
                    Color(90, 120, 150, 24),
                    Rect::new(0., 0., w * 0.38, h * 0.48),
                );
                if mode == "transition" {
                    let (sw, sh) = ((width / 5).max(1), (height / 4).max(1));
                    let mut instance = ShaderInstance::new(DITHER);
                    let effect = scene.append(
                        panel,
                        NodeKind::Image(image(&mut instance, sw, sh, 0.)),
                        fixed(sw as f32, sh as f32),
                    );
                    scene.set_transform(effect, Transform { x: 12., y: 12. });
                    shaders.push((effect, instance, sw, sh));
                }
                panels.push((panel, fill));
            }
        }
        if matches!(mode, "uniforms" | "dither" | "shader_resize") {
            let count = if mode == "dither" { 8 } else { 1 };
            let (sw, sh) = if count == 1 {
                (width / 2, height / 2)
            } else {
                (width / 4, height / 2)
            };
            for n in 0..count {
                let mut instance = ShaderInstance::new(DITHER);
                let node = scene.append(
                    root,
                    NodeKind::Image(image(&mut instance, sw, sh, 0.)),
                    fixed(sw as f32, sh as f32),
                );
                scene.set_transform(
                    node,
                    Transform {
                        x: (n % 4) as f32 * sw as f32,
                        y: (n / 4) as f32 * sh as f32,
                    },
                );
                shaders.push((node, instance, sw, sh));
            }
        }
        let chain = if mode == "chain" {
            let (sw, sh) = (width / 2, height / 2);
            let pixels: Vec<u8> = (0..sw * sh)
                .flat_map(|i| {
                    [
                        (i % sw * 255 / sw) as u8,
                        (i / sw * 255 / sh) as u8,
                        160,
                        255,
                    ]
                })
                .collect();
            let input = std::sync::Arc::new(ImageData::new(sw, sh, pixels).unwrap());
            let mut chain = EffectChain::new();
            let output = chain.render(input.clone(), &chain_stages(0)).unwrap();
            let node = scene.append(root, NodeKind::Image(output), fixed(sw as f32, sh as f32));
            Some((node, chain, input))
        } else {
            None
        };
        // Painted after filters, but intersecting them: source pixels remain unchanged.
        let cursor = rect(
            &mut scene,
            root,
            Color(250, 250, 255, 255),
            Rect::new(w * 0.4, h * 0.3, 2., 20.),
        );
        Self {
            scene,
            content,
            cursor,
            panels,
            shaders,
            chain,
        }
    }
    fn update(&mut self, gpu: &mut GpuRenderer, mode: &str, frame: usize, width: u32, height: u32) {
        match mode {
            "transition" => {
                // Four overlapping frosted cards resize/rotate while their dither
                // surfaces update at 20 Hz relative to 60 Hz UI frames.
                let phase = frame as f32 / 60.;
                for (n, (panel, _)) in self.panels.iter().enumerate() {
                    let t = (phase * 2. + n as f32 * 0.4).sin();
                    self.scene.set_paint_transform_origin(
                        *panel,
                        Affine::scale(0.85 + 0.15 * t, 0.9 + 0.1 * t)
                            .then(Affine::rotation(t * 0.12)),
                        [0.5, 0.5],
                    );
                }
                self.scene.set_transform(
                    self.content,
                    Transform {
                        x: 0.,
                        y: -((frame % 60) as f32) * 2.,
                    },
                );
                if frame.is_multiple_of(3) {
                    for (n, (node, instance, w, h)) in self.shaders.iter_mut().enumerate() {
                        self.scene.set_kind(
                            *node,
                            NodeKind::Image(image(instance, *w, *h, frame as f32 + n as f32 * 9.)),
                        );
                    }
                }
            }
            "overlap" => self.scene.set_effects(
                self.content,
                Effects {
                    opacity: if frame.is_multiple_of(2) { 0.8 } else { 1. },
                    ..Default::default()
                },
            ),
            "scroll" => self.scene.set_transform(
                self.content,
                Transform {
                    x: 0.,
                    y: -((frame % 60) as f32) * 2.,
                },
            ),
            "foreground" => self.scene.set_kind(
                self.cursor,
                NodeKind::Rect(if frame.is_multiple_of(2) {
                    Color(250, 250, 255, 0)
                } else {
                    Color(250, 250, 255, 255)
                }),
            ),
            "resize" => {
                let w = width - (frame % 8) as u32 * 8;
                self.scene.resize(w as f32, height as f32);
                gpu.resize(w, height);
                for (n, (panel, fill)) in self.panels.iter().enumerate() {
                    self.scene
                        .set_style(*panel, fixed(w as f32 * 0.38, height as f32 * 0.48));
                    self.scene
                        .set_style(*fill, fixed(w as f32 * 0.38, height as f32 * 0.48));
                    self.scene.set_transform(
                        *panel,
                        Transform {
                            x: w as f32 * (0.09 + n as f32 * 0.14),
                            y: height as f32 * (0.08 + n as f32 * 0.13),
                        },
                    );
                }
            }
            "shader_resize" => {
                for (node, instance, _, h) in &mut self.shaders {
                    let w = width / 2 - (frame % 8) as u32 * (width / 128).max(1);
                    self.scene
                        .set_kind(*node, NodeKind::Image(image(instance, w, *h, frame as f32)));
                    self.scene.set_style(*node, fixed(w as f32, *h as f32));
                }
            }
            "chain" => {
                let (node, chain, input) = self.chain.as_mut().unwrap();
                self.scene.set_kind(
                    *node,
                    NodeKind::Image(chain.render(input.clone(), &chain_stages(frame)).unwrap()),
                );
            }
            "uniforms" | "dither" => {
                for (n, (node, instance, w, h)) in self.shaders.iter_mut().enumerate() {
                    self.scene.set_kind(
                        *node,
                        NodeKind::Image(image(instance, *w, *h, frame as f32 + n as f32 * 9.)),
                    );
                }
            }
            _ => unreachable!(),
        }
    }
}
fn chain_stages(frame: usize) -> [EffectStage; 2] {
    [
        EffectStage::Blur { radius: 6. },
        EffectStage::Dither {
            levels: 2 + (frame % 7) as u32,
            cell_size: 2,
        },
    ]
}

fn profile(profile: GpuFrameProfile, stages: &mut BTreeMap<&str, Vec<f64>>) {
    stages.entry("total").or_default().push(profile.duration_ms);
    let mut sums = BTreeMap::<&str, f64>::new();
    for span in profile.spans {
        *sums.entry(span.label).or_default() += span.duration_ms;
    }
    for (label, ms) in sums {
        stages.entry(label).or_default().push(ms);
    }
}
fn add_stats(stats: GpuStats, totals: &mut BTreeMap<&str, u64>) {
    for (label, value) in [
        ("blur_passes", stats.blur_passes as u64),
        ("blur_cache_hits", stats.blur_cache_hits as u64),
        ("filtered_pixels", stats.filtered_pixels),
        ("shader_dispatches", stats.shader_dispatches as u64),
        (
            "effect_stage_cache_hits",
            stats.effect_stage_cache_hits as u64,
        ),
        (
            "shader_resource_allocations",
            stats.shader_resource_allocations as u64,
        ),
        (
            "blur_texture_allocations",
            stats.blur_texture_allocations as u64,
        ),
        (
            "vertex_buffer_allocations",
            stats.vertex_buffer_allocations as u64,
        ),
        (
            "layer_texture_allocations",
            stats.layer_texture_allocations as u64,
        ),
        (
            "paint_geometry_buffer_allocations",
            stats.paint_geometry_buffer_allocations as u64,
        ),
        ("draw_calls", stats.draw_calls as u64),
        ("render_passes", stats.render_passes as u64),
        ("damaged_pixels", stats.damaged_pixels),
        ("scroll_copies", stats.scroll_copies as u64),
        ("copied_pixels", stats.copied_pixels),
    ] {
        *totals.entry(label).or_default() += value;
    }
}
fn main() {
    let config = Config::default();
    let context = GpuContext::new().unwrap();
    let sizes = std::env::var("LUMINE_BENCH_SIZES").unwrap_or_else(|_| "1280x720".into());
    let sigmas = std::env::var("LUMINE_BENCH_SIGMAS").unwrap_or_else(|_| "6,24".into());
    let clock = Instant::now();
    let mut trace = Trace::default();
    for spec in sizes.split(',') {
        let (width, height) = spec.split_once('x').expect("WIDTHxHEIGHT");
        let (width, height): (u32, u32) = (width.parse().unwrap(), height.parse().unwrap());
        assert!(width >= 64 && height >= 64);
        for mode in [
            "overlap",
            "scroll",
            "foreground",
            "uniforms",
            "resize",
            "shader_resize",
            "dither",
            "chain",
            "transition",
        ] {
            if !config.includes(mode) {
                continue;
            }
            let blur = matches!(
                mode,
                "overlap" | "scroll" | "foreground" | "resize" | "transition"
            );
            let radii: Vec<f32> = if blur {
                sigmas.split(',').map(|s| s.parse().unwrap()).collect()
            } else {
                vec![0.]
            };
            for sigma in radii {
                assert!(sigma.is_finite() && sigma <= 64. && (sigma > 0. || !blur));
                for algorithm in [BlurAlgorithm::Gaussian, BlurAlgorithm::DualKawase]
                    .into_iter()
                    .take(if blur { 2 } else { 1 })
                {
                    run(
                        &config, &context, width, height, mode, sigma, algorithm, &mut trace, clock,
                    );
                }
            }
        }
    }
    if let Some(path) = config.trace {
        trace.write(&path).unwrap();
    }
}
#[allow(clippy::too_many_arguments)]
fn run(
    config: &Config,
    context: &GpuContext,
    width: u32,
    height: u32,
    mode: &str,
    sigma: f32,
    algorithm: BlurAlgorithm,
    trace: &mut Trace,
    clock: Instant,
) {
    let mut gpu = GpuRenderer::new_with_context(width, height, context).unwrap();
    gpu.set_blur_algorithm(algorithm);
    let timestamps = gpu.set_gpu_profiling(true);
    let mut workload = Workload::new(width, height, mode, sigma);
    let initial = workload.scene.flush();
    gpu.render(&workload.scene, &initial.damage).unwrap();
    gpu.wait_idle().unwrap();
    gpu.take_gpu_profiles();
    let mut stages = BTreeMap::<&str, Vec<f64>>::new();
    let mut gpu_stages = BTreeMap::<&str, Vec<f64>>::new();
    let mut totals = BTreeMap::new();
    let mut frames_with_gpu_work = 0;
    let mut measured_start = Instant::now();
    let mut dropped_before = gpu.dropped_gpu_profiles();
    let track = format!("{width}x{height}/{mode}/{algorithm:?}/{sigma}");
    for frame in 0..config.warmup + config.frames {
        if frame == config.warmup {
            gpu.wait_idle().unwrap();
            gpu.take_gpu_profiles();
            measured_start = Instant::now();
            dropped_before = gpu.dropped_gpu_profiles();
        }
        let start = Instant::now();
        workload.update(&mut gpu, mode, frame, width, height);
        let updated = Instant::now();
        let report = workload.scene.flush();
        let flushed = Instant::now();
        let stats = gpu.render(&workload.scene, &report.damage).unwrap();
        gpu.submit();
        let submitted = Instant::now();
        if config.in_flight == 1 || (frame + 1) % config.in_flight == 0 {
            gpu.wait_idle().unwrap();
        }
        let complete = Instant::now();
        let profiles = gpu.take_gpu_profiles();
        if frame < config.warmup {
            continue;
        }
        for (label, a, b) in [
            ("update", start, updated),
            ("flush", updated, flushed),
            ("encode_submit", flushed, submitted),
            ("cpu_frame", start, submitted),
            (
                if config.in_flight == 1 {
                    "completed_frame"
                } else {
                    "submission_batch_frame"
                },
                start,
                complete,
            ),
        ] {
            let ms = (b - a).as_secs_f64() * 1000.;
            stages.entry(label).or_default().push(ms);
            if config.trace.is_some() {
                trace.span(label, &track, (a - clock).as_secs_f64() * 1000., ms);
            }
        }
        for value in profiles {
            profile(value, &mut gpu_stages);
        }
        add_stats(stats, &mut totals);
        frames_with_gpu_work += usize::from(
            stats.render_passes > 0 || stats.shader_dispatches > 0 || stats.scroll_copies > 0,
        );
    }
    gpu.wait_idle().unwrap();
    for value in gpu.take_gpu_profiles() {
        profile(value, &mut gpu_stages);
    }
    if mode == "foreground" {
        assert_eq!(
            totals["filtered_pixels"], 0,
            "foreground damage must reuse filters"
        );
        assert!(totals["blur_cache_hits"] > 0);
    }
    if matches!(mode, "uniforms" | "dither") && config.warmup >= 2 {
        assert_eq!(
            totals["shader_resource_allocations"], 0,
            "animated uniforms must reuse shader resources after warmup"
        );
        assert!(totals["shader_dispatches"] > 0);
    }
    if mode == "chain" && config.warmup >= 2 {
        assert_eq!(
            totals["effect_stage_cache_hits"], config.frames as u64,
            "the unchanged blur prefix must be cached every frame"
        );
        assert_eq!(
            totals["shader_dispatches"], config.frames as u64,
            "only the animated dither suffix may dispatch"
        );
        assert_eq!(totals["shader_resource_allocations"], 0);
    }
    let elapsed = measured_start.elapsed().as_secs_f64();
    let caches = gpu.debug_cache_stats();
    let info = gpu.adapter_info();
    let gpu_timed_frames = gpu_stages.get("total").map_or(0, Vec::len);
    let dropped = gpu.dropped_gpu_profiles() - dropped_before;
    emit(serde_json::json!({
        "scene":"effects","workload":mode,"physical_size":[width,height],"scale":1.,
        "recorded_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        "sigma":sigma,"algorithm":format!("{algorithm:?}"),"panels":if sigma>0. {4} else {0},
        "surfaces":workload.shaders.len(),"adapter":info.name,"backend":format!("{:?}",info.backend),
        "driver":info.driver,"vendor":info.vendor,"device":info.device,
        "build":if cfg!(debug_assertions) {"debug"} else {"release"},
        "frames":config.frames,"warmup":config.warmup,"in_flight":config.in_flight,"target_hz":config.hz,
        "budget_ms":config.budget_ms(),"render_throughput_hz":config.frames as f64/elapsed,
        "gpu_timestamps_supported":timestamps,"gpu_profiles_dropped":dropped,
        "frames_with_gpu_work":frames_with_gpu_work,"gpu_timed_frames":gpu_timed_frames,
        "gpu_measurements_valid":timestamps && dropped == 0 && gpu_timed_frames == frames_with_gpu_work,
        "cpu":stages.into_iter().map(|(k,v)|(k,summary(&v,config.budget_ms()))).collect::<BTreeMap<_,_>>(),
        "gpu":gpu_stages.into_iter().map(|(k,v)|(k,summary(&v,config.budget_ms()))).collect::<BTreeMap<_,_>>(),
        "totals":totals,
        "per_frame":totals.iter().map(|(k,v)|(*k,*v as f64/config.frames as f64)).collect::<BTreeMap<_,_>>(),
        "retained_bytes":{"blur_cache":caches.blur_cache_bytes,"blur_scratch":caches.blur_scratch_bytes,
            "effect_chain_including_final_image_texture":caches.effect_chain_bytes,
            "shader_resources_including_image_textures":caches.shader_resource_bytes,"images":caches.image_bytes,
            "layers":caches.layer_bytes,"vertices":caches.vertex_buffer_bytes,"paint_geometry":caches.paint_geometry_bytes,"atlas":caches.atlas_bytes,
            "scroll_cache":caches.scroll_cache_bytes},
        "target_texture_bytes":width as u64*height as u64*4,
        "memory_note":"shader_resources and effect_chain include final textures also counted in images; do not sum these fields",
    }));
}
