//! Immutable decoded straight-alpha RGBA8 images shared between scene nodes.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
#[path = "effect_chain.rs"]
mod chain;
pub use chain::{EffectChain, EffectStage, FilteredImage};
pub(crate) fn next_identity() -> Result<u64, &'static str> {
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| "image identity exhausted")
}
/// Texture filtering at paint time. Pixel art and ordered dither use nearest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageSampling {
    #[default]
    Linear,
    Nearest,
}
#[derive(Clone, Debug)]
pub struct ImageData {
    id: u64,
    width: u32,
    height: u32,
    pixels: Arc<std::sync::OnceLock<Arc<[u8]>>>,
    procedural: Option<Arc<ProceduralImage>>,
    chain: Option<Arc<FilteredImage>>,
    transform: crate::affine::Affine,
    sampling: ImageSampling,
}
/// A compute shader producing premultiplied RGBA8, with a lazy straight-alpha
/// fallback for software consumers. Binding 0 is a read-only f32 storage buffer;
/// binding 1 is an rgba8unorm storage texture. One invocation covers one cell.
pub struct ProceduralImage {
    pub shader: &'static str,
    pub parameters: Vec<f32>,
    pub dispatch: [u32; 2],
    /// Stable resource identity; snapshots still have distinct `ImageData::id`s.
    pub instance: Option<u64>,
    fallback: Box<dyn Fn() -> Arc<[u8]> + Send + Sync>,
}
impl std::fmt::Debug for ProceduralImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProceduralImage")
    }
}
impl ImageData {
    pub fn new(
        width: u32,
        height: u32,
        pixels: impl Into<Arc<[u8]>>,
    ) -> Result<Self, &'static str> {
        let pixels = pixels.into();
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or("image size overflow")?;
        if width == 0 || height == 0 || pixels.len() != expected {
            return Err("image requires nonzero dimensions and width * height * 4 RGBA bytes");
        }
        let id = next_identity()?;
        Ok(Self {
            id,
            width,
            height,
            pixels: Arc::new(std::sync::OnceLock::from(pixels)),
            procedural: None,
            chain: None,
            transform: crate::affine::Affine::IDENTITY,
            sampling: ImageSampling::default(),
        })
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn pixels(&self) -> &[u8] {
        self.pixels.get_or_init(|| {
            if let Some(chain) = &self.chain {
                chain::fallback(chain)
            } else {
                (self.procedural.as_ref().unwrap().fallback)()
            }
        })
    }
    pub fn procedural(&self) -> Option<&ProceduralImage> {
        self.procedural.as_deref()
    }
    pub fn effect_chain(&self) -> Option<&FilteredImage> {
        self.chain.as_deref()
    }
    pub fn compute(
        width: u32,
        height: u32,
        shader: &'static str,
        parameters: Vec<f32>,
        dispatch: [u32; 2],
        fallback: impl Fn() -> Arc<[u8]> + Send + Sync + 'static,
    ) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || dispatch.contains(&0) {
            return Err("nonzero image dimensions required");
        }
        Ok(Self {
            id: next_identity()?,
            width,
            height,
            pixels: Arc::new(std::sync::OnceLock::new()),
            procedural: Some(Arc::new(ProceduralImage {
                shader,
                parameters,
                dispatch,
                instance: None,
                fallback: Box::new(fallback),
            })),
            chain: None,
            transform: crate::affine::Affine::IDENTITY,
            sampling: ImageSampling::default(),
        })
    }
    /// Share decoded pixels and GPU upload identity with a different paint
    /// transformation. The matrix acts around the allocated image's center;
    /// it does not change intrinsic size or surrounding layout. Non-finite
    /// matrices reset to identity. Singular matrices paint and hit-test empty.
    pub fn transformed(&self, transform: crate::affine::Affine) -> Self {
        Self {
            id: self.id,
            width: self.width,
            height: self.height,
            pixels: self.pixels.clone(),
            procedural: self.procedural.clone(),
            chain: self.chain.clone(),
            sampling: self.sampling,
            transform: if transform.is_finite() {
                transform
            } else {
                crate::affine::Affine::IDENTITY
            },
        }
    }
    pub fn transform(&self) -> crate::affine::Affine {
        self.transform
    }
    pub fn sampling(&self) -> ImageSampling {
        self.sampling
    }
    /// Change filtering while sharing decoded pixels and GPU upload identity.
    pub fn sampled(&self, sampling: ImageSampling) -> Self {
        Self {
            sampling,
            ..self.clone()
        }
    }
    pub fn paint_transform(&self, bounds: crate::scene::Rect) -> crate::affine::Affine {
        if self.transform == crate::affine::Affine::IDENTITY {
            return self.transform;
        }
        self.transform.around(
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height * 0.5,
        )
    }
    pub fn paint_bounds(&self, bounds: crate::scene::Rect) -> crate::scene::Rect {
        if self.transform == crate::affine::Affine::IDENTITY {
            return bounds;
        }
        if self.transform.inverse().is_none() {
            return crate::scene::Rect::default();
        }
        self.paint_transform(bounds).bounds(bounds)
    }
}
impl PartialEq for ImageData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.transform == other.transform && self.sampling == other.sampling
    }
}
impl Eq for ImageData {}

/// Explicit f32 storage-buffer layout for a compute shader. Read motion values
/// while constructing this type inside `image_signal`; snapshots remain immutable.
pub trait ShaderUniforms {
    fn encode(&self) -> Vec<f32>;
}
impl<const N: usize> ShaderUniforms for [f32; N] {
    fn encode(&self) -> Vec<f32> {
        self.to_vec()
    }
}
impl ShaderUniforms for Vec<f32> {
    fn encode(&self) -> Vec<f32> {
        self.clone()
    }
}
/// One shader surface with persistent GPU resources and lazy CPU fallbacks.
/// Use a separate instance for surfaces that display different uniforms at once.
pub struct ShaderInstance {
    id: u64,
    shader: &'static str,
    last: Option<Arc<ImageData>>,
}
impl ShaderInstance {
    pub fn new(shader: &'static str) -> Self {
        Self {
            id: next_identity().expect("shader identity exhausted"),
            shader,
            last: None,
        }
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn set_shader(&mut self, shader: &'static str) {
        if self.shader != shader {
            self.shader = shader;
            self.last = None;
        }
    }
    pub fn render(
        &mut self,
        width: u32,
        height: u32,
        uniforms: &impl ShaderUniforms,
        dispatch: [u32; 2],
        fallback: impl Fn() -> Arc<[u8]> + Send + Sync + 'static,
    ) -> Result<Arc<ImageData>, &'static str> {
        let parameters = uniforms.encode();
        if parameters.is_empty() || parameters.iter().any(|v| !v.is_finite()) {
            return Err("shader uniforms must be nonempty and finite");
        }
        if let Some(last) = &self.last {
            let p = last.procedural().unwrap();
            if last.width == width
                && last.height == height
                && p.parameters == parameters
                && p.dispatch == dispatch
            {
                return Ok(last.clone());
            }
        }
        let mut image =
            ImageData::compute(width, height, self.shader, parameters, dispatch, fallback)?;
        Arc::get_mut(image.procedural.as_mut().unwrap())
            .unwrap()
            .instance = Some(self.id);
        let image = Arc::new(image);
        self.last = Some(image.clone());
        Ok(image)
    }
}
