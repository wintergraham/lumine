//! Bounded retained SVG pixels, shared by GPU and software renderers.
//!
//! Rasters are keyed by what they show (source, tint and pixel size), not by
//! node: every copy of an icon shares one raster (and one atlas cell), and a
//! row a virtual list mounts again finds its icons already rasterized.
use crate::GpuError;
use std::{collections::HashMap, hash::{Hash, Hasher}, sync::Arc};
use lumine::{
    image::ImageData,
    scene::{NodeId, NodeKind, Rect, Scene},
    svg::SvgData,
};
const MAX_ENTRIES: usize = 1024;
/// Rasters no node shows any more, kept (least recently used first out) for
/// rows that scroll back into view.
const UNUSED_BUDGET: usize = 1024 * 1024;
/// Independent copies used for exact source comparison, released with entries.
const SOURCE_BUDGET: usize = 8 * 1024 * 1024;
#[derive(Clone, PartialEq, Eq)]
struct Key {
    source_hash: u64,
    source_bytes: Arc<[u8]>,
    tint: Option<[u8; 4]>,
    width: u32,
    height: u32,
    geometry: [u32; 4],
}
impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Equality also compares the bytes, so a fingerprint collision cannot
        // make different SVG sources share pixels.
        (self.source_hash, self.tint, self.width, self.height, self.geometry).hash(state);
    }
}
struct Entry {
    image: Arc<ImageData>,
    used: u64,
    /// Nodes showing this raster.
    users: u32,
}
struct Shown {
    source_id: u64,
    key: Key,
    transformed: Arc<ImageData>,
}
#[derive(Default)]
pub struct SvgCache {
    entries: HashMap<Key, Entry>,
    nodes: HashMap<NodeId, Shown>,
    bytes: usize,
    source_bytes: usize,
    clock: u64,
    rasterizations: u64,
    animating: bool,
}
impl SvgCache {
    pub(crate) fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }
    pub fn retain(&mut self, scene: &Scene) {
        let entries = &mut self.entries;
        self.nodes.retain(|node, shown| {
            let live = scene.contains(*node) && matches!(scene.kind(*node), NodeKind::Svg(_));
            if !live && let Some(entry) = entries.get_mut(&shown.key) {
                entry.users -= 1;
            }
            live
        });
        let mut unused: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.users == 0)
            .map(|(key, e)| (e.used, e.image.pixels().len(), key.clone()))
            .collect();
        unused.sort_unstable_by_key(|&(used, ..)| std::cmp::Reverse(used));
        let mut kept = 0;
        for (_, bytes, key) in unused {
            let retained = bytes + key.source_bytes.len();
            if kept + retained > UNUSED_BUDGET {
                self.entries.remove(&key);
                self.bytes -= bytes;
                self.source_bytes -= key.source_bytes.len();
            } else {
                kept += retained;
            }
        }
    }
    pub fn image_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.values().map(|e| e.image.id())
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn rasterizations(&self) -> u64 {
        self.rasterizations
    }
    pub fn get(
        &mut self,
        node: NodeId,
        source: &Arc<SvgData>,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Arc<ImageData>, GpuError> {
        if ![width, height, scale]
            .iter()
            .all(|v| v.is_finite() && *v > 0.)
        {
            return Err(GpuError("invalid SVG size".into()));
        }
        let (width, height) = (
            (width * scale).ceil() as u32,
            (height * scale).ceil() as u32,
        );
        self.get_raster(
            node,
            source,
            width,
            height,
            [width as f32, height as f32, 0., 0.],
        )
    }
    /// Keep resting icons exact on the device grid. During animation, bounded
    /// quarter-pixel phases avoid a new raster for every float position.
    pub(crate) fn get_at(
        &mut self,
        node: NodeId,
        source: &Arc<SvgData>,
        bounds: Rect,
        scale: f32,
    ) -> Result<(Arc<ImageData>, Rect), GpuError> {
        let phase = |position: f32| {
            if self.animating {
                (position * scale * 4.).round() / 4.
            } else {
                position * scale
            }
        };
        let x = phase(bounds.x);
        let y = phase(bounds.y);
        let geometry = [
            bounds.width * scale,
            bounds.height * scale,
            x - x.floor(),
            y - y.floor(),
        ];
        let width = (geometry[0] + geometry[2]).ceil() as u32;
        let height = (geometry[1] + geometry[3]).ceil() as u32;
        let image = self.get_raster(node, source, width, height, geometry)?;
        Ok((
            image,
            Rect::new(
                x.floor() / scale,
                y.floor() / scale,
                width as f32 / scale,
                height as f32 / scale,
            ),
        ))
    }
    fn get_raster(
        &mut self,
        node: NodeId,
        source: &Arc<SvgData>,
        width: u32,
        height: u32,
        geometry: [f32; 4],
    ) -> Result<Arc<ImageData>, GpuError> {
        if (u64::from(width) * u64::from(height))
            .checked_mul(4)
            .is_none_or(|bytes| bytes > 32 * 1024 * 1024)
        {
            return Err(GpuError("SVG raster exceeds 32 MiB".into()));
        }
        self.clock = self.clock.wrapping_add(1);
        // Reconstructed controls can own new SVG sources with identical bytes.
        // Keep independent comparison bytes, never the caller's source storage.
        let previous_source = self.nodes.get(&node)
            .filter(|shown| shown.source_id == source.source_id())
            .map(|shown| (shown.key.source_hash, shown.key.source_bytes.clone()));
        let (source_hash, source_bytes) = previous_source
            .unwrap_or_else(|| {
                let mut hash = rustc_hash::FxHasher::default();
                source.bytes().hash(&mut hash);
                (hash.finish(), Arc::from(source.bytes()))
            });
        let key = Key {
            source_hash,
            source_bytes,
            tint: source.tint().map(|c| [c.0, c.1, c.2, c.3]),
            width,
            height,
            geometry: geometry.map(f32::to_bits),
        };
        if let Some(shown) = self.nodes.get_mut(&node)
            && shown.key == key
            && let Some(entry) = self.entries.get_mut(&key)
        {
            shown.source_id = source.source_id();
            entry.used = self.clock;
            if shown.transformed.transform() != source.transform() {
                shown.transformed = Arc::new(entry.image.transformed(source.transform()));
            }
            return Ok(shown.transformed.clone());
        }
        if !self.entries.contains_key(&key) {
            let image = self.rasterize(source, width, height, geometry)?;
            let bytes = image.pixels().len();
            while (self.bytes + bytes > 32 * 1024 * 1024
                || self.source_bytes + key.source_bytes.len() > SOURCE_BUDGET
                || self.entries.len() >= MAX_ENTRIES)
                && !self.entries.is_empty()
            {
                let victim = self.entries.iter().min_by_key(|(_, e)| e.used).unwrap().0.clone();
                self.bytes -= self.entries.remove(&victim).unwrap().image.pixels().len();
                self.source_bytes -= victim.source_bytes.len();
                self.nodes.retain(|_, shown| shown.key != victim);
            }
            self.bytes += bytes;
            self.source_bytes += key.source_bytes.len();
            self.entries.insert(
                key.clone(),
                Entry {
                    image,
                    used: 0,
                    users: 0,
                },
            );
        }
        // Nodes share the entry's comparison storage, including cache hits from
        // newly reconstructed sources. Do not retain a source copy per node.
        let key = self.entries.get_key_value(&key).expect("raster").0.clone();
        let entry = self.entries.get_mut(&key).expect("raster");
        entry.used = self.clock;
        entry.users += 1;
        let transformed = if source.transform() == entry.image.transform() {
            entry.image.clone()
        } else {
            Arc::new(entry.image.transformed(source.transform()))
        };
        let previous = self.nodes.insert(
            node,
            Shown {
                source_id: source.source_id(),
                key,
                transformed: transformed.clone(),
            },
        );
        if let Some(previous) = previous
            && let Some(entry) = self.entries.get_mut(&previous.key)
        {
            entry.users -= 1;
        }
        Ok(transformed)
    }
    fn rasterize(
        &mut self,
        source: &SvgData,
        width: u32,
        height: u32,
        geometry: [f32; 4],
    ) -> Result<Arc<ImageData>, GpuError> {
        let image = crate::assets::decode_svg_at(source.bytes(), width, height, geometry)?;
        self.rasterizations += 1;
        let Some(color) = source.tint() else {
            return Ok(image);
        };
        let mut pixels = image.pixels().to_vec();
        for p in pixels.as_chunks_mut::<4>().0 {
            p[0] = color.0;
            p[1] = color.1;
            p[2] = color.2;
            p[3] = ((u16::from(p[3]) * u16::from(color.3) + 127) / 255) as u8;
        }
        Ok(Arc::new(
            ImageData::new(width, height, pixels).map_err(|e| GpuError(e.into()))?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_fingerprint_collisions_compare_exact_bytes() {
        let key = |bytes: &[u8]| Key {
            source_hash: 7,
            source_bytes: Arc::from(bytes),
            tint: None,
            width: 1,
            height: 1,
            geometry: [0; 4],
        };
        let mut entries = HashMap::new();
        entries.insert(key(b"first SVG"), 1);
        entries.insert(key(b"second SVG"), 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.get(&key(b"first SVG")), Some(&1));
        assert_eq!(entries.get(&key(b"second SVG")), Some(&2));
    }
    #[test]
    fn reconstructed_sources_share_only_equal_bytes_and_comparison_storage() {
        let mut scene = Scene::new(20., 20.);
        let make = |fill: &str| Arc::new(SvgData::new(format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10' fill='{fill}'/></svg>"
        ).into_bytes()).unwrap());
        let red = make("red");
        let reconstructed = make("red");
        let blue = make("blue");
        assert_ne!(red.source_id(), reconstructed.source_id());
        let a = scene.append(scene.root(), NodeKind::Svg(red.clone()), Default::default());
        let b = scene.append(scene.root(), NodeKind::Svg(reconstructed.clone()), Default::default());
        let mut cache = SvgCache::default();
        let first = cache.get(a, &red, 10., 10., 1.).unwrap();
        let second = cache.get(b, &reconstructed, 10., 10., 1.).unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(cache.rasterizations(), 1);
        assert!(Arc::ptr_eq(&cache.nodes[&a].key.source_bytes, &cache.nodes[&b].key.source_bytes));
        let changed = cache.get(b, &blue, 10., 10., 1.).unwrap();
        assert_ne!(changed.id(), first.id());
        assert_ne!(changed.pixels(), first.pixels());
        assert_eq!(cache.rasterizations(), 2);
    }
    #[test]
    fn large_comparison_sources_are_bounded_and_released_when_unused() {
        let mut scene = Scene::new(10., 10.);
        let mut cache = SvgCache::default();
        let mut nodes = Vec::new();
        for index in 0..3 {
            let prefix = format!("<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><!--{index}");
            let suffix = "--></svg>";
            let source = Arc::new(SvgData::new(format!("{prefix}{}{suffix}",
                " ".repeat(4 * 1024 * 1024 - prefix.len() - suffix.len())).into_bytes()).unwrap());
            let node = scene.append(scene.root(), NodeKind::Svg(source.clone()), Default::default());
            cache.get(node, &source, 1., 1., 1.).unwrap();
            nodes.push(node);
            assert!(cache.source_bytes <= SOURCE_BUDGET);
        }
        assert!(!cache.nodes.contains_key(&nodes[0]), "old comparison storage was not evicted");
        assert!(cache.nodes.contains_key(&nodes[2]), "new source must still render");
        for node in nodes { scene.remove(node); }
        cache.retain(&scene);
        assert!(cache.entries.is_empty(), "unused oversized sources exceeded the retention budget");
        assert_eq!(cache.source_bytes, 0);
    }
    #[test]
    fn animated_translation_has_bounded_rasters_at_each_dpi() {
        let mut scene = Scene::new(100., 100.);
        let source = Arc::new(SvgData::new(&b"<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><circle cx='8' cy='8' r='6'/></svg>"[..]).unwrap());
        let node = scene.append(
            scene.root(),
            NodeKind::Svg(source.clone()),
            Default::default(),
        );
        for scale in [1., 1.5, 2.] {
            let mut cache = SvgCache::default();
            cache.set_animating(true);
            for frame in 0..1000 {
                let bounds = Rect::new(frame as f32 * 0.0137, frame as f32 * 0.0091, 16., 16.);
                let (_, raster) = cache.get_at(node, &source, bounds, scale).unwrap();
                assert!((raster.x * scale - bounds.x * scale).abs() <= 1.125);
                assert!(raster.x + raster.width >= bounds.x + bounds.width - 0.125 / scale);
            }
            assert!(
                cache.rasterizations() <= 16,
                "translation produced {} rasters at {scale} DPI",
                cache.rasterizations()
            );
        }
    }
    #[test]
    fn unchanged_svg_reuses_transformed_handle_without_rasterization() {
        let mut scene = Scene::new(100., 100.);
        let source=Arc::new(SvgData::new(&b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10' fill='red'/></svg>"[..]).unwrap());
        let node = scene.append(
            scene.root(),
            NodeKind::Svg(source.clone()),
            Default::default(),
        );
        let mut cache = SvgCache::default();
        let first = cache.get(node, &source, 20., 20., 1.).unwrap();
        let second = cache.get(node, &source, 20., 20., 1.).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let rotated = Arc::new(source.transformed(lumine::affine::Affine::rotation(0.5)));
        let third = cache.get(node, &rotated, 20., 20., 1.).unwrap();
        assert!(!Arc::ptr_eq(&first, &third));
        let fourth = cache.get(node, &rotated, 20., 20., 1.).unwrap();
        assert!(Arc::ptr_eq(&third, &fourth));
        assert_eq!(cache.rasterizations(), 1);
    }
    #[test]
    fn removed_svg_source_drops_before_next_paint_and_transform_keeps_pixels() {
        let mut scene = Scene::new(100., 100.);
        let bytes:Arc<[u8]>=Arc::from(&b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10'/></svg>"[..]);
        let source = Arc::new(SvgData::new(bytes.clone()).unwrap());
        let weak = Arc::downgrade(&source);
        let transformed = Arc::new(source.transformed(lumine::affine::Affine::rotation(0.3)));
        let node = scene.append(
            scene.root(),
            NodeKind::Svg(source.clone()),
            Default::default(),
        );
        let mut cache = SvgCache::default();
        let first = cache.get(node, &source, 20., 20., 1.).unwrap();
        scene.set_kind(node, NodeKind::Svg(transformed.clone()));
        drop(source);
        assert!(weak.upgrade().is_none());
        let second = cache.get(node, &transformed, 20., 20., 1.).unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(cache.rasterizations(), 1);
        scene.remove(node);
        drop(transformed);
        assert_eq!(
            Arc::strong_count(&bytes),
            1,
            "cache must not keep SVG byte storage alive"
        );
        assert!(
            cache.bytes() > 0,
            "test runs before next render retention pass"
        );
    }
    #[test]
    fn tiny_svg_rasters_obey_entry_count_and_lru_limit() {
        let mut scene = Scene::new(100., 100.);
        let mut cache = SvgCache::default();
        let mut first = None;
        for index in 0..=MAX_ENTRIES {
            let bytes = format!("<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><!--{index}--></svg>");
            let source = Arc::new(SvgData::new(bytes.into_bytes()).unwrap());
            let node = scene.append(
                scene.root(),
                NodeKind::Svg(source.clone()),
                Default::default(),
            );
            first.get_or_insert(node);
            cache.get(node, &source, 1., 1., 1.).unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert_eq!(cache.bytes(), MAX_ENTRIES * 4);
        assert!(!cache.nodes.contains_key(&first.unwrap()));
        assert_eq!(cache.source_bytes, cache.entries.keys().map(|key|key.source_bytes.len()).sum::<usize>());
    }
    #[test]
    fn equal_icons_share_a_raster_that_outlives_a_remount() {
        let mut scene = Scene::new(100., 100.);
        let source = Arc::new(
            SvgData::new(&b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10'/></svg>"[..])
                .unwrap(),
        );
        let red = Arc::new(source.tinted(lumine::scene::Color(255, 0, 0, 255)));
        let mut cache = SvgCache::default();
        let mount = |scene: &mut Scene, source: &Arc<SvgData>| {
            scene.append(
                scene.root(),
                NodeKind::Svg(source.clone()),
                Default::default(),
            )
        };
        let (a, b, c) = (
            mount(&mut scene, &red),
            mount(&mut scene, &red),
            mount(&mut scene, &source),
        );
        let first = cache.get(a, &red, 10., 10., 2.).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &cache.get(b, &red, 10., 10., 2.).unwrap()
        ));
        cache.get(c, &source, 10., 10., 2.).unwrap();
        cache.get(c, &source, 10., 10., 1.).unwrap();
        assert_eq!(cache.rasterizations(), 3, "tint and size are distinct");
        // A row scrolled out and back in: its icon is not rasterized again.
        scene.remove(a);
        scene.remove(b);
        cache.retain(&scene);
        assert_eq!(cache.entries.len(), 3, "a small unused raster is kept");
        let again = mount(&mut scene, &red);
        assert_eq!(
            cache.get(again, &red, 10., 10., 2.).unwrap().id(),
            first.id()
        );
        assert_eq!(cache.rasterizations(), 3);
        // Past the budget, rasters no node shows are released.
        scene.remove(again);
        cache.retain(&scene);
        let big = mount(&mut scene, &source);
        cache.get(big, &source, 600., 600., 1.).unwrap();
        scene.remove(big);
        cache.retain(&scene);
        assert_eq!(
            cache.entries.len(),
            3,
            "the large unused raster is released"
        );
        assert!(cache.bytes() < 4096);
        assert_eq!(
            cache.bytes(),
            cache
                .entries
                .values()
                .map(|e| e.image.pixels().len())
                .sum::<usize>()
        );
    }
}
