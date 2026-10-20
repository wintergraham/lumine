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

/// Separate actual audit executions and compiler-reported header/child-command
/// observations. Byte identity with production artifacts is reproducibility
/// evidence; the original production command stream is still unobserved.
/// This receipt always states unqualified and executionClosureComplete=false.
pub fn source_paint_execution_audit_json() -> &'static str {
    include_str!(concat!(
        env!("OUT_DIR"),
        "/source-paint-execution-audit.json"
    ))
}
/// Declared digest of the embedded audit bytes. Independent verification must
/// recompute it and recheck every bounded referenced file before and after use.
pub fn source_paint_execution_audit_sha256() -> Option<&'static str> {
    let value = include_str!(concat!(
        env!("OUT_DIR"),
        "/source-paint-execution-audit.sha256"
    ))
    .trim();
    (value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v)))
    .then_some(value)
}

/// Safety gate and pre/post selected-cc/configured-tool observations for the
/// original cc debug command stream. Always unqualified; not a kernel trace.
pub fn source_paint_original_cc_output_context_json() -> &'static str {
    include_str!(concat!(
        env!("OUT_DIR"),
        "/source-paint-original-cc-output.json"
    ))
}
/// Cargo writes this actual build-script stdout file after build.rs exits and
/// before compiling this crate. No build-script prediction of its final digest
/// participates. The actual trusted inspecting caller must SHA256 these exact
/// bytes, parse the bounded reviewed cc command/status grammar between markers,
/// and verify the context plus referenced artifacts before/after use.
/// Includes ordinary Cargo metadata outside the markers; never a shell script.
pub fn source_paint_original_cc_cargo_output_bytes() -> Option<&'static [u8]> {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/../output"));
    (!bytes.is_empty() && bytes.len() <= 8 * 1024 * 1024).then_some(bytes.as_slice())
}
