//! Observations of the actual renderer and allocated retained output texture.
//! These values do not qualify source paint or describe the monitor's color profile.

/// Adapter identity returned by wgpu for the device used by this renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterInfo {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub device_type: String,
    pub backend: String,
    pub driver: String,
    pub driver_info: String,
}

/// Read from the allocated texture, rather than a requested render descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureInfo {
    pub format: String,
    pub size: [u32; 3],
    pub sample_count: u32,
    pub mip_level_count: u32,
    pub dimension: String,
    pub usage_bits: u32,
}

/// Configured presentation surface. This is distinct from retained pixel storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceInfo {
    pub format: String,
    pub size: [u32; 2],
    pub alpha_mode: String,
    pub present_mode: String,
    pub usage_bits: u32,
    pub view_formats: Vec<String>,
    pub desired_maximum_frame_latency: u32,
    pub converts_to_straight_alpha: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RendererInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub adapter: AdapterInfo,
    pub retained_target: TextureInfo,
    /// Effective renderer scale, including its validation/clamping policy.
    pub scale_factor: f32,
    /// Retained texture extent divided by effective renderer scale.
    pub logical_viewport: [f64; 2],
    /// Renderer byte encoding contract, not a measured monitor/ICC color space.
    pub retained_color_encoding: &'static str,
    pub retained_alpha: &'static str,
    pub background_rgba: [u8; 4],
    pub isolated_layer: bool,
    /// None for offscreen renderers; never synthesized from window options.
    pub surface: Option<SurfaceInfo>,
}

impl crate::GpuRenderer {
    /// Snapshot the adapter/device identity and the current retained texture.
    /// No candidate, source contract, environment override or requested window
    /// metadata participates in this observation.
    pub fn renderer_info(&self) -> RendererInfo {
        let size = self.target.size();
        RendererInfo {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            adapter: AdapterInfo {
                name: self.info.name.clone(),
                vendor: self.info.vendor,
                device: self.info.device,
                device_type: format!("{:?}", self.info.device_type),
                backend: format!("{:?}", self.info.backend),
                driver: self.info.driver.clone(),
                driver_info: self.info.driver_info.clone(),
            },
            retained_target: TextureInfo {
                format: format!("{:?}", self.target.format()),
                size: [size.width, size.height, size.depth_or_array_layers],
                sample_count: self.target.sample_count(),
                mip_level_count: self.target.mip_level_count(),
                dimension: format!("{:?}", self.target.dimension()),
                usage_bits: self.target.usage().bits(),
            },
            scale_factor: self.scale,
            logical_viewport: [
                f64::from(size.width) / f64::from(self.scale),
                f64::from(size.height) / f64::from(self.scale),
            ],
            // These are the renderer's established storage semantics. The
            // non-sRGB texture format alone cannot establish byte encoding.
            retained_color_encoding: "srgb-encoded-bytes",
            retained_alpha: "premultiplied",
            background_rgba: [
                self.background.0,
                self.background.1,
                self.background.2,
                self.background.3,
            ],
            isolated_layer: self.in_layer,
            surface: self.config.as_ref().map(|config| SurfaceInfo {
                format: format!("{:?}", config.format),
                size: [config.width, config.height],
                alpha_mode: format!("{:?}", config.alpha_mode),
                present_mode: format!("{:?}", config.present_mode),
                usage_bits: config.usage.bits(),
                view_formats: config
                    .view_formats
                    .iter()
                    .map(|format| format!("{format:?}"))
                    .collect(),
                desired_maximum_frame_latency: config.desired_maximum_frame_latency,
                converts_to_straight_alpha: self.present_unpremultiplies,
            }),
        }
    }
}
