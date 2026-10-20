//! Manual offscreen diagnostics only. A source-observed raster partition is
//! separate from a source capability, target observation, and UI acceptance.
use crate::renderer_info::RendererInfo;
use lumine::source_paint::{SourcePaintCapability, SourcePaintPolicy};
use std::num::NonZeroU64;

/// Identifies this manual diagnostic algorithm, never a qualification receipt.
pub const MANUAL_SOURCE_RASTER_ALGORITHM_REVISION: &str =
    "source-skia154-convex-rle-rectclip-h2/trace-raster-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceRasterTile {
    /// Actual source raster allocation, including its observed border texels.
    pub raster_bounds: [u32; 4],
    /// Disjoint physical target pixels presented from this allocation.
    pub owned_bounds: [u32; 4],
}
#[derive(Clone, Debug)]
pub struct ManualSourceRasterPlan {
    owner: NonZeroU64,
    contract_sha256: [u8; 32],
    source_trace_sha256: [u8; 32],
    source_geometry_material_plan_sha256: [u8; 32],
    target: RendererInfo,
    tiles: Vec<SourceRasterTile>,
}
fn edges(rect: [u32; 4]) -> Option<[u32; 4]> {
    (rect[2] > 0 && rect[3] > 0).then_some([
        rect[0],
        rect[1],
        rect[0].checked_add(rect[2])?,
        rect[1].checked_add(rect[3])?,
    ])
}
impl ManualSourceRasterPlan {
    /// The native test owner supplies hashes and tiles from immutable original
    /// source observations. This constructor checks structure, not authenticity
    /// or qualification. It grants no SourcePaintCapability and accepts no PNG.
    pub fn from_owner_observed_trace(
        owner: NonZeroU64,
        contract_sha256: [u8; 32],
        source_trace_sha256: [u8; 32],
        source_geometry_material_plan_sha256: [u8; 32],
        actual_target: RendererInfo,
        tiles: Vec<SourceRasterTile>,
    ) -> Result<Self, &'static str> {
        let [width, height, depth] = actual_target.retained_target.size;
        if [
            contract_sha256,
            source_trace_sha256,
            source_geometry_material_plan_sha256,
        ]
        .into_iter()
        .any(|hash| hash == [0; 32])
            || width == 0
            || height == 0
            || width > 8191
            || height > 8191
            || u64::from(width) * u64::from(height) > 32 * 1024 * 1024
            || depth != 1
            || actual_target.retained_target.sample_count != 1
            || actual_target.retained_target.mip_level_count != 1
            || actual_target.retained_target.dimension != "D2"
            || !matches!(
                actual_target.retained_target.format.as_str(),
                "Bgra8Unorm" | "Rgba8Unorm"
            )
            || actual_target.retained_color_encoding != "srgb-encoded-bytes"
            || actual_target.retained_alpha != "premultiplied"
            || actual_target.surface.is_some()
            || actual_target.isolated_layer
            || !actual_target.scale_factor.is_finite()
            || actual_target.scale_factor <= 0.
            || actual_target.logical_viewport
                != [
                    f64::from(width) / f64::from(actual_target.scale_factor),
                    f64::from(height) / f64::from(actual_target.scale_factor),
                ]
            || tiles.is_empty()
            || tiles.len() > 4096
        {
            return Err("unsupported manual source raster identity or target");
        }
        let mut area = 0u64;
        for (index, tile) in tiles.iter().enumerate() {
            let raster = edges(tile.raster_bounds).ok_or("invalid source raster extent")?;
            let owned = edges(tile.owned_bounds).ok_or("invalid owned raster extent")?;
            // Source border allocations may overhang the visible content; all
            // emitted owned pixels must remain inside the actual retained target.
            if raster[0] >= width
                || raster[1] >= height
                || raster[2] > 8191
                || raster[3] > 8191
                || owned[0] < raster[0]
                || owned[1] < raster[1]
                || owned[2] > raster[2]
                || owned[3] > raster[3]
                || owned[2] > width
                || owned[3] > height
            {
                return Err("source raster bounds escape the observed target");
            }
            for previous in &tiles[..index] {
                let other = edges(previous.owned_bounds).ok_or("invalid prior raster extent")?;
                if owned[0] < other[2]
                    && other[0] < owned[2]
                    && owned[1] < other[3]
                    && other[1] < owned[3]
                {
                    return Err("overlapping owned source raster pixels");
                }
            }
            area += u64::from(tile.owned_bounds[2]) * u64::from(tile.owned_bounds[3]);
        }
        if area != u64::from(width) * u64::from(height) {
            return Err("source raster partition does not cover the target");
        }
        Ok(Self {
            owner,
            contract_sha256,
            source_trace_sha256,
            source_geometry_material_plan_sha256,
            target: actual_target,
            tiles,
        })
    }
    pub fn source_trace_sha256(&self) -> [u8; 32] {
        self.source_trace_sha256
    }
    pub fn source_geometry_material_plan_sha256(&self) -> [u8; 32] {
        self.source_geometry_material_plan_sha256
    }
    pub(crate) fn same_raster_algorithm(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.contract_sha256 == other.contract_sha256
            && self.source_trace_sha256 == other.source_trace_sha256
            && self.target == other.target
            && self.tiles == other.tiles
    }
    pub(crate) fn target_matches(&self, target: &RendererInfo) -> bool {
        self.target == *target
    }
    pub(crate) fn applies_to(&self, capability: Option<SourcePaintCapability>) -> bool {
        capability.is_some_and(|cap| {
            cap.owner == self.owner
                && cap.contract_sha256 == self.contract_sha256
                && cap.policy == SourcePaintPolicy::SkiaSoftwareSrgb8KnownPremultipliedTarget
        })
    }
    pub(crate) fn tiles(&self) -> &[SourceRasterTile] {
        &self.tiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer_info::{AdapterInfo, TextureInfo};
    fn target() -> RendererInfo {
        RendererInfo {
            os: "linux",
            arch: "x86_64",
            adapter: AdapterInfo {
                name: "test".into(),
                vendor: 0,
                device: 0,
                device_type: "Cpu".into(),
                backend: "Vulkan".into(),
                driver: "test".into(),
                driver_info: "test".into(),
            },
            retained_target: TextureInfo {
                format: "Bgra8Unorm".into(),
                size: [32, 24, 1],
                sample_count: 1,
                mip_level_count: 1,
                dimension: "D2".into(),
                usage_bits: 19,
            },
            scale_factor: 2.,
            logical_viewport: [16., 12.],
            retained_color_encoding: "srgb-encoded-bytes",
            retained_alpha: "premultiplied",
            background_rgba: [0; 4],
            isolated_layer: false,
            surface: None,
        }
    }
    fn plan(
        target: RendererInfo,
        tiles: Vec<SourceRasterTile>,
    ) -> Result<ManualSourceRasterPlan, &'static str> {
        ManualSourceRasterPlan::from_owner_observed_trace(
            NonZeroU64::new(1).unwrap(),
            [1; 32],
            [2; 32],
            [3; 32],
            target,
            tiles,
        )
    }
    fn tiles() -> Vec<SourceRasterTile> {
        vec![
            SourceRasterTile {
                raster_bounds: [0, 0, 18, 24],
                owned_bounds: [0, 0, 17, 24],
            },
            SourceRasterTile {
                raster_bounds: [16, 0, 18, 24],
                owned_bounds: [17, 0, 15, 24],
            },
        ]
    }
    #[test]
    fn independent_owned_partition_is_complete_and_owner_target_bound() {
        let plan = plan(target(), tiles()).unwrap();
        assert!(plan.target_matches(&target()));
        let capability = SourcePaintCapability {
            owner: NonZeroU64::new(1).unwrap(),
            contract_sha256: [1; 32],
            policy: SourcePaintPolicy::SkiaSoftwareSrgb8KnownPremultipliedTarget,
        };
        assert!(plan.applies_to(Some(capability)));
        assert!(!plan.applies_to(None));
        assert!(!plan.applies_to(Some(SourcePaintCapability {
            owner: NonZeroU64::new(2).unwrap(),
            ..capability
        })));
        assert!(!plan.applies_to(Some(SourcePaintCapability {
            contract_sha256: [4; 32],
            ..capability
        })));
        assert!(!plan.applies_to(Some(SourcePaintCapability {
            policy: SourcePaintPolicy::SkiaSoftwareSrgb8OpaqueBackdrop,
            ..capability
        })));
        let mut changed = target();
        changed.background_rgba = [1, 2, 3, 255];
        assert!(!plan.target_matches(&changed));
        changed = target();
        changed.adapter.driver_info = "changed".into();
        assert!(!plan.target_matches(&changed));
        changed = target();
        changed.scale_factor = 1.;
        assert!(!plan.target_matches(&changed));
    }
    #[test]
    fn gaps_overlaps_empty_escape_overflow_and_bad_targets_reject() {
        for mutation in 0..6 {
            let mut tiles = tiles();
            match mutation {
                0 => tiles[1].owned_bounds[0] += 1,
                1 => {
                    tiles[1].owned_bounds[0] -= 1;
                    tiles[1].owned_bounds[2] += 1;
                }
                2 => tiles[0].owned_bounds[2] = 0,
                3 => tiles[1].owned_bounds[2] += 1,
                4 => tiles[0].raster_bounds[2] = 1,
                _ => tiles[1].raster_bounds[2] = u32::MAX,
            }
            assert!(plan(target(), tiles).is_err());
        }
        for mutation in 0..7 {
            let mut target = target();
            match mutation {
                0 => target.isolated_layer = true,
                1 => target.scale_factor = f32::NAN,
                2 => target.retained_alpha = "straight",
                3 => target.retained_target.format = "Bgra8UnormSrgb".into(),
                4 => target.retained_target.sample_count = 4,
                5 => target.logical_viewport = [1., 1.],
                _ => target.retained_target.size[2] = 2,
            }
            assert!(plan(target, tiles()).is_err());
        }
    }
}
