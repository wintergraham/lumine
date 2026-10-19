//! One-shot draw attribution using the existing profiler query/readback pool.
//! Diagnostic pass partitioning preserves draw batches, order and pixels, but
//! adds load/store boundaries. Its timings are for attribution, not acceptance.
use super::*;
use lumine::scene::PaintItem;

pub const MAX_BATCHES: u32 = 64;
const WINDOW_BATCHES: u32 = 48;
const MAX_PASSES: usize = 192;
const MAX_SOURCES: usize = 16;

#[derive(Clone, Copy)]
pub(super) struct Source {
    node: NodeId,
    kind: &'static str,
    clip_count: u32,
}
impl Source {
    pub fn new(item: &PaintItem<'_>, clip_count: u32) -> Self {
        Self {
            node: item.id,
            clip_count,
            kind: if item.isolated {
                "isolated_layer"
            } else {
                match item.kind {
                    NodeKind::Canvas(_) => "cached_canvas",
                    NodeKind::Image(_) => "image",
                    NodeKind::Text { .. } | NodeKind::RichText { .. } => "text",
                    NodeKind::Svg(_) => "svg",
                    NodeKind::Quad(_) => "quad",
                    NodeKind::Rect(_) => "rect",
                    NodeKind::Panel { .. } => "panel",
                    _ => "other",
                }
            },
        }
    }
}

/// Source ranges are clipped to the submitted instance range. Node IDs use the
/// scene's Debug representation, including generation, rather than layer names.
#[derive(Clone, Debug)]
pub struct DrawSource {
    pub scene_node_id: String,
    pub kind: &'static str,
    pub instances: std::ops::Range<u32>,
    pub clip_count: u32,
    pub logical_bounds: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct DrawBatch {
    pub instances: std::ops::Range<u32>,
    pub geometry_offset: u32,
    pub shading: &'static str,
    pub image: Option<u64>,
    pub image_size: Option<[u32; 2]>,
    pub layer: Option<String>,
    pub physical_scissor: [u32; 4],
    /// A conservative bounding box, not covered fragments or shader invocations.
    pub physical_coverage_bounds: Option<[u32; 4]>,
    pub gaussian_shadow_instances: usize,
    pub nearest_image_instances: usize,
    pub masked_instances: usize,
    pub sources: Vec<DrawSource>,
    pub omitted_sources: usize,
}
#[derive(Clone, Debug)]
pub struct DrawPass {
    /// Index in GpuFrameProfile::spans. None means no query capacity remained.
    pub span_index: Option<usize>,
    pub stage: &'static str,
    /// Actual submitted batch order across root regions and blur segments.
    pub batches: std::ops::Range<u32>,
    /// Present only for a single selected batch. A group is never per-draw time.
    pub batch: Option<DrawBatch>,
}
#[derive(Clone, Debug)]
pub struct DrawDiagnostics {
    pub first_batch: u32,
    pub total_batches: u32,
    pub selected_batches: u32,
    pub omitted_batches: u32,
    pub omitted_pass_records: u32,
    pub missing_timestamp_pairs: u32,
    pub passes: Vec<DrawPass>,
}
impl DrawDiagnostics {
    pub(super) fn new(first_batch: u32) -> Self {
        Self {
            first_batch,
            total_batches: 0,
            selected_batches: 0,
            omitted_batches: 0,
            omitted_pass_records: 0,
            missing_timestamp_pairs: 0,
            passes: Vec::new(),
        }
    }
    pub(super) fn select(&mut self, large_image: bool) -> (u32, bool) {
        let index = self.total_batches;
        self.total_batches += 1;
        let in_window = index >= self.first_batch && index - self.first_batch < WINDOW_BATCHES;
        // Reserve the window even if large images occur before it.
        let extras = self
            .selected_batches
            .saturating_sub(index.saturating_sub(self.first_batch).min(WINDOW_BATCHES));
        let selected = in_window || (large_image && extras < MAX_BATCHES - WINDOW_BATCHES);
        if selected {
            self.selected_batches += 1;
        } else {
            self.omitted_batches += 1;
        }
        (index, selected)
    }
    pub(super) fn record(&mut self, pass: DrawPass) {
        if pass.span_index.is_none() {
            self.missing_timestamp_pairs += 1;
        }
        if self.passes.len() < MAX_PASSES {
            self.passes.push(pass);
        } else {
            self.omitted_pass_records += 1;
        }
    }
    #[cfg(feature = "benchmark")]
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        json!({
            "mode":"ordered-pass-partition", "timingPerturbed":true,
            "scope":"root redraw; isolated layer internals, clear, blur and presentation are not source draw batches",
            "coverageMeaning":"conservative physical bounding boxes; not fragment counts",
            "limits":{"windowBatches":WINDOW_BATCHES,"extraLargeImageBatches":MAX_BATCHES-WINDOW_BATCHES,
                "passRecords":MAX_PASSES,"sourcesPerBatch":MAX_SOURCES,"queryReservePairs":64},
            "firstBatch":self.first_batch,"totalBatches":self.total_batches,
            "selectedBatches":self.selected_batches,"omittedBatches":self.omitted_batches,
            "omittedPassRecords":self.omitted_pass_records,"missingTimestampPairs":self.missing_timestamp_pairs,
            "passes":self.passes.iter().map(|p| json!({"spanIndex":p.span_index,"stage":p.stage,
                "batchRange":[p.batches.start,p.batches.end],"batch":p.batch.as_ref().map(|b|json!({
                    "instanceRange":[b.instances.start,b.instances.end],"geometryOffset":b.geometry_offset,
                    "shading":b.shading,"imageId":b.image,"imageSize":b.image_size,"layerNodeId":b.layer,
                    "physicalScissor":b.physical_scissor,"physicalCoverageBounds":b.physical_coverage_bounds,
                    "physicalCoverageUpperBoundPixels":b.physical_coverage_bounds.map(|[_,_,w,h]|u64::from(w)*u64::from(h)),
                    "gaussianShadowInstances":b.gaussian_shadow_instances,"nearestImageInstances":b.nearest_image_instances,
                    "maskedInstances":b.masked_instances,"omittedSources":b.omitted_sources,
                    "sources":b.sources.iter().map(|s|json!({"sceneNodeId":s.scene_node_id,"kind":s.kind,
                        "instanceRange":[s.instances.start,s.instances.end],"clipCount":s.clip_count,
                        "logicalBounds":s.logical_bounds})).collect::<Vec<_>>()
                }))})).collect::<Vec<_>>()
        })
    }
}

/// The same batching iterator is used in normal and diagnostic encoding.
/// No allocation or diagnostic metadata is needed for the normal path.
pub(super) fn batches(
    draws: &[Draw],
    region: Rect,
    first: u32,
    unblended: bool,
) -> impl Iterator<Item = Batch> + '_ {
    let mut draws = draws.iter();
    let mut open: Option<Batch> = None;
    std::iter::from_fn(move || {
        for draw in draws.by_ref() {
            if !region.intersects(draw.bounds) || draw.end <= draw.start.max(first) {
                continue;
            }
            let start = draw.start.max(first);
            let Some(rect) = region.intersection(draw.clip) else {
                if open.is_some() {
                    return open.take();
                }
                continue;
            };
            let shading = match draw.shading {
                Shading::Opaque if !unblended => Shading::Basic,
                shading => shading,
            };
            let source = (draw.image, draw.layer);
            if let Some(batch) = &mut open
                && batch.rect == rect
                && batch.end == start
                && batch.source == source
                && batch.shading == shading
                && batch.geometry_offset == draw.geometry_offset
            {
                batch.end = draw.end;
                continue;
            }
            let next = Batch {
                geometry_offset: draw.geometry_offset,
                geometry: draw.geometry.clone(),
                rect,
                source,
                shading,
                start,
                end: draw.end,
            };
            if let Some(done) = open.replace(next) {
                return Some(done);
            }
        }
        open.take()
    })
}

fn batch_sources(batch: &Batch, draws: &[Draw]) -> (Vec<DrawSource>, usize, Option<Rect>) {
    let mut sources = Vec::new();
    let mut omitted_sources = 0;
    let mut bounds: Option<Rect> = None;
    for draw in draws
        .iter()
        .filter(|d| d.start < batch.end && d.end > batch.start)
    {
        if let Some(area) = draw.bounds.intersection(batch.rect) {
            bounds = Some(bounds.map_or(area, |b| b.union(area)));
        }
        if let Some(source) = draw.diagnostic_source {
            if sources.len() < MAX_SOURCES {
                sources.push(DrawSource {
                    scene_node_id: format!("{:?}", source.node),
                    kind: source.kind,
                    instances: draw.start.max(batch.start)..draw.end.min(batch.end),
                    clip_count: source.clip_count,
                    logical_bounds: [
                        draw.bounds.x,
                        draw.bounds.y,
                        draw.bounds.width,
                        draw.bounds.height,
                    ],
                });
            } else {
                omitted_sources += 1;
            }
        }
    }
    (sources, omitted_sources, bounds)
}

impl GpuRenderer {
    fn diagnostic_batch(&self, batch: &Batch, draws: &[Draw], quads: &[Quad]) -> DrawBatch {
        let (sources, omitted_sources, bounds) = batch_sources(batch, draws);
        let quads = &quads[batch.start as usize..batch.end as usize];
        let (x, y, w, h) = scissor(batch.rect, self.scale, self.width, self.height).unwrap();
        DrawBatch {
            instances: batch.start..batch.end,
            geometry_offset: batch.geometry_offset,
            shading: match selected_shading(batch.shading, self.regular_full_shader) {
                Shading::Full => "full",
                Shading::RegularFull => "regular_full",
                Shading::Basic => "basic",
                Shading::Opaque => "opaque",
            },
            image: batch.source.0,
            image_size: batch
                .source
                .0
                .and_then(|id| self.image_textures.get(&id))
                .map(|t| [t.width(), t.height()]),
            layer: batch.source.1.map(|id| format!("{id:?}")),
            physical_scissor: [x, y, w, h],
            physical_coverage_bounds: bounds
                .and_then(|b| scissor(b, self.scale, self.width, self.height))
                .map(|(x, y, w, h)| [x, y, w, h]),
            gaussian_shadow_instances: quads
                .iter()
                .filter(|q| gaussian_shadow(q))
                .count(),
            nearest_image_instances: quads
                .iter()
                .filter(|q| q.options[1] > 1.5 && q.options[3] > 0.5)
                .count(),
            masked_instances: quads
                .iter()
                .filter(|q| q.mask[2] != 0. || q.mask[3] != 0.)
                .count(),
            sources,
            omitted_sources,
        }
    }
    fn diagnostic_stamp(&self, label: &'static str) -> Option<profiling::PassStamp> {
        self.profiler
            .borrow_mut()
            .as_mut()
            .and_then(|p| p.draw_pass(label))
    }
    fn diagnostic_record(
        &self,
        stamp: Option<profiling::PassStamp>,
        stage: &'static str,
        batches: std::ops::Range<u32>,
        batch: Option<DrawBatch>,
    ) {
        if let Some(d) = self
            .profiler
            .borrow_mut()
            .as_mut()
            .and_then(|p| p.draw_diagnostics())
        {
            d.record(DrawPass {
                span_index: stamp.map(|s| s.span_index()),
                stage,
                batches,
                batch,
            });
        }
    }
    fn diagnostic_render_pass<'a>(
        &self,
        encoder: &'a mut wgpu::CommandEncoder,
        stamp: &Option<profiling::PassStamp>,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("diagnostic damage slice"),
            timestamp_writes: stamp.as_ref().map(|s| s.writes()),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        })
    }
    pub(super) fn encode_diagnostic_segment(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        vertices: &wgpu::Buffer,
        quads: &[Quad],
        regions: &[(Rect, (u32, u32, u32, u32))],
        draws: &[Draw],
        first: &[u32],
        clear: bool,
        vertical: Option<VerticalBlur>,
        stage: &'static str,
        stats: &mut GpuStats,
    ) {
        let labels = match stage {
            "pre_blur" => ["pre_blur_setup", "pre_blur_batch", "pre_blur_batch_group"],
            "post_blur" => [
                "post_blur_setup",
                "post_blur_batch",
                "post_blur_batch_group",
            ],
            _ => ["repaint_setup", "repaint_batch", "repaint_batch_group"],
        };
        if let Some(blur) = vertical {
            let stamp = self.diagnostic_stamp("post_blur_composite");
            let mut pass = self.diagnostic_render_pass(encoder, &stamp);
            Self::encode_vertical(&mut pass, self.blur_vertical(&blur), &blur, stats);
            drop(pass);
            stats.render_passes += 1;
            self.diagnostic_record(stamp, "blur_composite", 0..0, None);
        }
        for (index, (region, (x, y, w, h))) in regions.iter().enumerate() {
            if clear && first[index] == 0 {
                let stamp = self.diagnostic_stamp(labels[0]);
                let mut pass = self.diagnostic_render_pass(encoder, &stamp);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_bind_group(0, &self.bind, &[]);
                pass.set_bind_group(1, self.viewport_bind.as_ref(), &[]);
                pass.set_bind_group(2, &self.geometry.bind, &[0]);
                pass.set_pipeline(self.context.inner.quads.clear(self.split_shading));
                pass.set_scissor_rect(*x, *y, *w, *h);
                pass.draw(0..6, 0..1);
                drop(pass);
                stats.draw_calls += 1;
                stats.render_passes += 1;
                self.diagnostic_record(stamp, "clear", 0..0, None);
            }
            let unblended = region.width * region.height * self.scale * self.scale >= SPLIT_PIXELS;
            let mut batches = batches(draws, *region, first[index], unblended)
                .filter(|b| scissor(b.rect, self.scale, self.width, self.height).is_some())
                .map(|b| {
                    let large_image = b.source.0.is_some()
                        && draws.iter().any(|d| {
                            d.start < b.end
                                && d.end > b.start
                                && d.bounds.intersection(b.rect).is_some_and(|r| {
                                    r.width * r.height * self.scale * self.scale >= 262_144.
                                })
                        });
                    let (ordinal, selected) = self
                        .profiler
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .draw_diagnostics()
                        .unwrap()
                        .select(large_image);
                    (b, ordinal, selected)
                })
                .peekable();
            while let Some((batch, ordinal, selected)) = batches.next() {
                let metadata = selected.then(|| self.diagnostic_batch(&batch, draws, quads));
                let stamp = self.diagnostic_stamp(labels[if selected { 1 } else { 2 }]);
                let mut pass = self.diagnostic_render_pass(encoder, &stamp);
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_bind_group(1, self.viewport_bind.as_ref(), &[]);
                let mut current = None;
                let pipelines = |s| self.context.inner.quads.get(s);
                self.encode_batch(&mut pass, &pipelines, &mut current, batch, stats);
                let mut end = ordinal + 1;
                if !selected {
                    while batches.peek().is_some_and(|(_, _, selected)| !selected) {
                        let (batch, ordinal, _) = batches.next().unwrap();
                        self.encode_batch(&mut pass, &pipelines, &mut current, batch, stats);
                        end = ordinal + 1;
                    }
                }
                drop(pass);
                stats.render_passes += 1;
                self.diagnostic_record(stamp, stage, ordinal..end, metadata);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draw(start: u32, end: u32) -> Draw {
        Draw {
            diagnostic_source: None,
            transform: lumine::affine::Affine::IDENTITY,
            geometry_offset: 0,
            geometry_hash: 0,
            geometry: None,
            shading: Shading::Full,
            blur: None,
            blur_mask: None,
            start,
            end,
            clip: Rect::new(0., 0., 100., 100.),
            bounds: Rect::new(0., 0., 100., 100.),
            image: None,
            layer: None,
        }
    }
    #[test]
    fn retained_draw_specialization_preserves_geometry_and_mixed_shadow_fallback() {
        let plain = Quad {
            color: [1.; 4],
            ..<Quad as bytemuck::Zeroable>::zeroed()
        };
        let transformed = Quad {
            options: [0., 2., 1., 1.],
            shape: [1., 0., 0.4, 1.],
            ..plain
        };
        let sharp = Quad {
            shape: [4., 0., 0.001, 0.],
            ..plain
        };
        let gaussian = Quad {
            shape: [4., 0., 0.01, 0.],
            ..plain
        };
        let quads = [plain, transformed, sharp, gaussian, plain];
        for split in [false, true] {
            for end in [3, 5] {
                let mut source = draw(0, end);
                source.geometry_offset = 256;
                source.geometry_hash = 17;
                source.transform = lumine::affine::Affine::rotation(0.3);
                source.bounds = Rect::new(20., 30., 70., 80.);
                let mut draws = Vec::new();
                push_draw(&mut draws, &quads, source.clone(), split, 2.);
                assert_eq!(
                    draws.len(),
                    1,
                    "retained draws must keep their original range"
                );
                let actual = &draws[0];
                assert_eq!((actual.start, actual.end), (source.start, source.end));
                assert_eq!(actual.bounds, source.bounds);
                assert_eq!(actual.transform, source.transform);
                assert_eq!(actual.geometry_offset, source.geometry_offset);
                assert_eq!(actual.geometry_hash, source.geometry_hash);
                assert_eq!(
                    actual.shading,
                    if split && end == 3 {
                        Shading::RegularFull
                    } else {
                        Shading::Full
                    }
                );
            }
        }
        let mut draws = Vec::new();
        push_draw(&mut draws, &quads, draw(1, 5), true, 2.);
        assert_eq!(
            draws.iter().map(|d| (d.start, d.end, d.shading)).collect::<Vec<_>>(),
            vec![
                (1, 3, Shading::RegularFull),
                (3, 4, Shading::Full),
                (4, 5, Shading::Opaque)
            ]
        );
    }
    #[test]
    fn batches_preserve_order_merging_clip_flush_and_geometry_binding() {
        let region = Rect::new(0., 0., 100., 100.);
        let mut draws = vec![
            draw(0, 2),
            draw(2, 5),
            draw(5, 6),
            draw(5, 8),
            draw(8, 9),
            draw(9, 10),
        ];
        // Missing clip must flush an open batch without painting this source.
        draws[2].clip = Rect::new(200., 200., 10., 10.);
        draws[4].geometry_offset = 256;
        draws[5].image = Some(7);
        let batches = batches(&draws, region, 1, true).collect::<Vec<_>>();
        assert_eq!(
            batches
                .iter()
                .map(|b| (b.start, b.end, b.geometry_offset, b.source.0))
                .collect::<Vec<_>>(),
            vec![
                (1, 5, 0, None),
                (5, 8, 0, None),
                (8, 9, 256, None),
                (9, 10, 0, Some(7))
            ]
        );
        draws[0].shading = Shading::Opaque;
        assert_eq!(
            super::batches(&draws, region, 0, false)
                .next()
                .unwrap()
                .shading,
            Shading::Basic
        );
    }
    #[test]
    fn source_mapping_crops_instance_ranges_and_bounds_metadata() {
        let scene = Scene::new(100., 100.);
        let source = Source {
            node: scene.root(),
            kind: "cached_canvas",
            clip_count: 3,
        };
        let mut draws = (0..20)
            .map(|i| {
                let mut d = draw(i * 2, i * 2 + 2);
                d.diagnostic_source = Some(source);
                d
            })
            .collect::<Vec<_>>();
        let batch = batches(&draws, Rect::new(0., 0., 100., 100.), 1, true)
            .next()
            .unwrap();
        let (sources, omitted, bounds) = batch_sources(&batch, &draws);
        assert_eq!(sources.len(), MAX_SOURCES);
        assert_eq!(omitted, 4);
        assert_eq!(sources[0].instances, 1..2);
        assert_eq!(sources[1].instances, 2..4);
        assert_eq!(sources[0].scene_node_id, format!("{:?}", scene.root()));
        assert_eq!(sources[0].clip_count, 3);
        assert_eq!(bounds, Some(Rect::new(0., 0., 100., 100.)));
        draws[0].diagnostic_source = None;
        assert_eq!(batch_sources(&batch, &draws).0[0].instances, 2..4);
    }
    #[test]
    fn selection_is_bounded_and_keeps_window_after_early_large_images() {
        let mut d = DrawDiagnostics::new(100);
        let selected = (0..500)
            .filter_map(|_| {
                let (index, selected) = d.select(true);
                selected.then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(&selected[..16], &(0..16).collect::<Vec<_>>());
        assert_eq!(&selected[16..], &(100..148).collect::<Vec<_>>());
        assert_eq!(d.total_batches, 500);
        assert_eq!(d.selected_batches, MAX_BATCHES);
        assert_eq!(d.omitted_batches, 436);
        for _ in 0..MAX_PASSES + 1 {
            d.record(DrawPass {
                span_index: None,
                stage: "post_blur",
                batches: 3..8,
                batch: None,
            });
        }
        assert_eq!(d.passes.len(), MAX_PASSES);
        assert_eq!(d.omitted_pass_records, 1);
        assert_eq!(d.missing_timestamp_pairs, MAX_PASSES as u32 + 1);
        assert!(d.passes[0].batch.is_none());
        assert_eq!(d.passes[0].batches, 3..8);
    }
}
