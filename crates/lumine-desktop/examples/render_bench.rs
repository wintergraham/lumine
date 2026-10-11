//! Standard offscreen rendering workloads, using the public component API.
//! cargo run --release -p lumine-desktop --example render_bench
use std::{collections::BTreeMap, time::Instant};
use lumine::{
    compose::prelude::*,
    scene::{Color, Effects},
    widgets::Ui,
};
use lumine_desktop::accessibility::AccessibilityTree;
use lumine_gpu::{
    GpuRenderer,
    benchmark::{Config, Trace, emit, summary},
};
fn main() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}
fn run() {
    let config = Config::default();
    let rows: usize = std::env::var("LUMINE_BENCH_ROWS")
        .unwrap_or_else(|_| "400".into())
        .parse()
        .unwrap();
    assert!((80..=10000).contains(&rows));
    let sizes = std::env::var("LUMINE_BENCH_SIZES")
        .unwrap_or_else(|_| "1920x1350@1.5,3840x2160@1.5,5120x2880@2".into());
    let mut trace = Trace::default();
    let clock = Instant::now();
    for spec in sizes.split(',') {
        let (size, scale) = spec.split_once('@').expect("physical WIDTHxHEIGHT@SCALE");
        let (w, h) = size.split_once('x').unwrap();
        let (pw, ph, scale): (u32, u32, f32) = (
            w.parse().unwrap(),
            h.parse().unwrap(),
            scale.parse().unwrap(),
        );
        assert!(pw > 0 && ph > 0 && scale.is_finite() && scale > 0.);
        let (width, height) = (pw as f32 / scale, ph as f32 / scale);
        for mode in [
            "idle",
            "vertical",
            "horizontal",
            "diagonal",
            "fractional",
            "text",
            "paint",
            "layout",
            "blur",
            "images",
            "isolated",
            "virtual_list",
            "resize",
        ] {
            if !config.includes(mode) {
                continue;
            }
            let mut gpu = GpuRenderer::new(pw, ph).unwrap();
            gpu.set_scale_factor(scale);
            gpu.set_background(Color(24, 28, 35, 255));
            let timestamps = gpu.set_gpu_profiling(true);
            if let Ok(split) = std::env::var("LUMINE_BENCH_SPLIT_SHADING") {
                gpu.debug_split_shading(split != "0");
            }
            if let Ok(enabled) = std::env::var("LUMINE_BENCH_OPAQUE_INTERIORS") {
                gpu.debug_opaque_interiors(enabled != "0");
            }
            if let Ok(enabled) = std::env::var("LUMINE_BENCH_PHASE_CACHE") {
                gpu.set_scroll_phase_cache(enabled != "0");
            }
            let mut ui = Ui::new(width, height);
            gpu.install_text(&mut ui.scene.borrow_mut());
            let x = ui.signal(300_f32);
            let y = ui.signal(300_f32);
            let message = ui.signal("Text shaping and streaming benchmark".to_owned());
            let opacity = ui.signal(1_f32);
            let box_width = ui.signal(90_f32);
            let mut content = column().w(width + 400.).gap(4.).bg(rgb(0x181c23));
            for row_index in 0..rows {
                let mut item = row()
                    .h(32.)
                    .gap(12.)
                    .items_center()
                    .bg(if row_index % 2 == 0 {
                        rgb(0x222936)
                    } else {
                        rgb(0x181c23)
                    })
                    .child(
                        text(format!(
                            "Row {row_index:05}: retained text, nested clipping, UI controls"
                        ))
                        .text_size(14.),
                    );
                item = item.child(div().h(16.).rounded(4.).bg(rgb(0x639cff)).reactive_style({
                    let opacity = opacity.clone();
                    let box_width = box_width.clone();
                    move || Styles::new().w(box_width.get()).opacity(opacity.get())
                }));
                if mode == "images" {
                    let pixels = [80_u8, 160, 240, 255].repeat(32 * 32);
                    let image = lumine::image::ImageData::new(32, 32, pixels).unwrap();
                    item = item.child(lumine::compose::image("icon", image.into()).size(24., 24.));
                }
                content = content.child(item.isolated(mode == "isolated"));
            }
            let stream = text_signal({
                let message = message.clone();
                move || message.get()
            })
            .h(28.)
            .text_size(14.);
            let scrolling = if mode == "virtual_list" {
                lumine::compose::virtual_list(
                    y.clone(),
                    32.,
                    2,
                    || 100000,
                    |i| i,
                    |_, i, _| row().h(32.).child(text(format!("Virtual row {i:06}"))),
                )
                .w_full()
                .h_full()
            } else {
                scroll_x(x.clone())
                    .w_full()
                    .h_full()
                    .child(scroll(y.clone()).w(width + 400.).h_full().child(content))
            };
            let root = ui.mount(
                overlay()
                    .w_full()
                    .h_full()
                    .text_color(rgb(0xe0e7ee))
                    .child(scrolling)
                    .child(stream.absolute().top(8.).left(12.))
                    .child(
                        div()
                            .id("effect")
                            .absolute()
                            .left(60.)
                            .top(60.)
                            .size(260., 160.)
                            .bg(rgba(0x30608060)),
                    ),
            );
            let effect = root.find("effect").unwrap();
            if mode == "blur" {
                ui.scene.borrow_mut().set_effects(
                    effect,
                    Effects {
                        blur_radius: 8.,
                        ..Default::default()
                    },
                );
            }
            if mode == "layout" {
                x.set(0.);
                y.set(0.);
            }
            ui.prepare_frame();
            let initial = ui.scene.borrow_mut().flush();
            gpu.render(&ui.scene.borrow(), &initial.damage).unwrap();
            gpu.wait_idle().unwrap();
            gpu.take_gpu_profiles();
            let mut accessibility = AccessibilityTree::new();
            let mut stages: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
            let mut gpu_stages: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
            let mut totals = [0_u64; 12];
            let mut frames_with_gpu_work = 0;
            let mut measured_start = Instant::now();
            let mut dropped_before = gpu.dropped_gpu_profiles();
            for i in 0..config.warmup + config.frames {
                if i == config.warmup {
                    gpu.wait_idle().unwrap();
                    gpu.take_gpu_profiles();
                    measured_start = Instant::now();
                    dropped_before = gpu.dropped_gpu_profiles();
                }
                let start = Instant::now();
                let offset = (i % 60) as f32;
                match mode {
                    "vertical" | "virtual_list" | "isolated" => {
                        y.set(300. + offset * 12.);
                    }
                    "horizontal" => {
                        x.set(100. + offset * 3.);
                    }
                    "diagonal" => {
                        x.set(100. + offset * 3.);
                        y.set(300. + offset * 12.);
                    }
                    "fractional" => {
                        y.set(300. + offset * 1.25);
                    }
                    "text" => {
                        message.set(format!(
                            "Frame {i}: new text shapes, shared glyphs and reusable allocations."
                        ));
                    }
                    "paint" | "images" | "blur" => {
                        opacity.set(if i % 2 == 0 { 0.4 } else { 1. });
                    }
                    "layout" => {
                        box_width.set(90. + (i % 7) as f32);
                    }
                    "resize" => {
                        ui.scene.borrow_mut().resize(width - (i % 8) as f32, height);
                        gpu.resize(pw - ((i % 8) as f32 * scale) as u32, ph);
                    }
                    _ => {}
                }
                ui.prepare_frame();
                let updated = Instant::now();
                let report = ui.scene.borrow_mut().flush();
                let flushed = Instant::now();
                let stats = gpu.render(&ui.scene.borrow(), &report.damage).unwrap();
                gpu.submit();
                let encoded = Instant::now();
                accessibility.update(
                    &ui.scene.borrow(),
                    &ui.semantics.borrow(),
                    ui.input.focused(),
                    "Profiling",
                    scale as f64,
                );
                let accessible = Instant::now();
                if config.in_flight == 1 || (i + 1) % config.in_flight == 0 {
                    gpu.wait_idle().unwrap();
                }
                let complete = Instant::now();
                let profiles = gpu.take_gpu_profiles();
                if i >= config.warmup {
                    frames_with_gpu_work += usize::from(
                        stats.render_passes > 0
                            || stats.shader_dispatches > 0
                            || stats.scroll_copies > 0,
                    );
                    for (label, a, b) in [
                        ("input_layout", start, updated),
                        ("flush", updated, flushed),
                        ("encode_submit", flushed, encoded),
                        ("accessibility", encoded, accessible),
                        (
                            if config.in_flight == 1 {
                                "completed_frame"
                            } else {
                                "cpu_frame"
                            },
                            start,
                            if config.in_flight == 1 {
                                complete
                            } else {
                                accessible
                            },
                        ),
                    ] {
                        let ms = (b - a).as_secs_f64() * 1000.;
                        stages.entry(label).or_default().push(ms);
                        if config.trace.is_some() {
                            trace.span(
                                label,
                                &format!("{spec}/{mode}"),
                                (a - clock).as_secs_f64() * 1000.,
                                ms,
                            );
                        }
                    }
                    for profile in profiles {
                        collect_profile(profile, &mut gpu_stages);
                    }
                    for (sum, value) in totals.iter_mut().zip([
                        stats.instances as u64,
                        stats.draw_calls as u64,
                        stats.damaged_pixels,
                        stats.scroll_copies as u64,
                        stats.copied_pixels,
                        stats.layer_repaints as u64,
                        stats.shaped_nodes as u64,
                        stats.glyph_uploads as u64,
                        stats.geometry_rebuilds as u64,
                        stats.vertex_buffer_allocations as u64,
                        stats.blur_passes as u64,
                        stats.scroll_phase_hits as u64,
                    ]) {
                        *sum += value;
                    }
                }
            }
            gpu.wait_idle().unwrap();
            for profile in gpu.take_gpu_profiles() {
                collect_profile(profile, &mut gpu_stages);
            }
            let elapsed = measured_start.elapsed().as_secs_f64();
            let caches = gpu.debug_cache_stats();
            let info = gpu.adapter_info();
            let gpu_timed_frames = gpu_stages.get("total").map_or(0, Vec::len);
            let dropped = gpu.dropped_gpu_profiles() - dropped_before;
            let mut record = serde_json::json!({
                "scene":"standard","workload":mode,"logical_size":[width,height],"physical_size":[pw,ph],
                "scale":scale,"rows":rows,"adapter":info.name,"backend":format!("{:?}",info.backend),
                "driver":info.driver,"vendor":info.vendor,"device":info.device,
                "budget_ms":config.budget_ms(),"target_hz":config.hz,"in_flight":config.in_flight,
                "build": if cfg!(debug_assertions) {"debug"} else {"release"},
                "gpu_timestamps_supported":timestamps,"gpu_profiles_dropped":dropped,
                "frames_with_gpu_work":frames_with_gpu_work,"gpu_timed_frames":gpu_timed_frames,
                "gpu_measurements_valid":timestamps && dropped == 0 && gpu_timed_frames == frames_with_gpu_work,
                "render_throughput_hz":config.frames as f64 / elapsed,
                "instances_per_frame":totals[0] as f64/config.frames as f64,
                "draws_per_frame":totals[1] as f64/config.frames as f64,
                "damaged_pixels_per_frame":totals[2] as f64/config.frames as f64,
                "scroll_copies":totals[3],"copied_pixels_per_frame":totals[4] as f64/config.frames as f64,
                "layer_repaints":totals[5],"shaped_nodes":totals[6],"glyph_uploads":totals[7],
                "scroll_phase_hits":totals[11],
                "geometry_rebuilds":totals[8],"vertex_buffer_allocations":totals[9],"blur_passes":totals[10],
                "cache_bytes":{"layers":caches.layer_bytes,"atlas":caches.atlas_bytes,
                    "scroll_phase":caches.scroll_cache_bytes,
                    "images":caches.image_bytes,"vertices":caches.vertex_buffer_bytes,"shaped":caches.shaped_bytes},
                "gpu":gpu_stages.into_iter().map(|(label,values)|(label,summary(&values,config.budget_ms()))).collect::<BTreeMap<_,_>>()
            });
            for (label, values) in stages {
                record[label] = summary(&values, config.budget_ms());
            }
            emit(record);
        }
    }
    if let Some(path) = config.trace {
        trace.write(&path).unwrap();
    }
}
fn collect_profile(
    profile: lumine_gpu::profiling::GpuFrameProfile,
    stages: &mut BTreeMap<&str, Vec<f64>>,
) {
    stages.entry("total").or_default().push(profile.duration_ms);
    let mut per_frame: BTreeMap<&str, f64> = BTreeMap::new();
    for span in profile.spans {
        *per_frame.entry(span.label).or_default() += span.duration_ms;
    }
    for (label, ms) in per_frame {
        stages.entry(label).or_default().push(ms);
    }
}
