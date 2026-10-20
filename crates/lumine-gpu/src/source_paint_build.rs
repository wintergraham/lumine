//! Build-time observations of the native source-paint raster subset.
//! An observed recipe is not qualification or permission to select a policy.

/// Actual recipe emitted by this crate's build script and embedded in this
/// binary. Independent trusted build verification must inspect these bytes,
/// verify their file/tool/artifact digests and compare an approved descriptor.
/// No caller-supplied environment or requested digest participates at runtime.
pub fn source_paint_build_recipe_json() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/source-paint-build-recipe.json"))
}

/// Digest of those actual JSON bytes, observed by the build script. Trusted
/// verification must recompute it from `source_paint_build_recipe_json()`;
/// the declared digest alone is neither evidence nor policy authority.
pub fn source_paint_build_recipe_sha256() -> Option<&'static str> {
    let value = include_str!(concat!(
        env!("OUT_DIR"),
        "/source-paint-build-recipe.sha256"
    ))
    .trim();
    (value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v)))
    .then_some(value)
}
