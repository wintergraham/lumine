mod source_paint_build_recipe;

fn tool_observation(tool: &cc::Tool) -> source_paint_build_recipe::Tool {
    let command = tool.to_command();
    source_paint_build_recipe::Tool {
        path: tool.path().to_owned(),
        arguments: tool.args().to_vec(),
        command_program: command.get_program().to_owned(),
        command_arguments: command.get_args().map(std::ffi::OsStr::to_owned).collect(),
        environment: tool.env().to_vec(),
    }
}

fn main() {
    println!("cargo:rerun-if-changed=src/source_skia154.cpp");
    let mut build = cc::Build::new();
    build
        .cargo_debug(false)
        .cpp(true)
        .std("c++17")
        .flag_if_supported("-ffp-contract=off")
        .file("src/source_skia154.cpp");
    let supported = build
        .is_flag_supported("-ffp-contract=off")
        .unwrap_or(false);
    let before = tool_observation(&build.get_compiler());
    let snapshot = source_paint_build_recipe::before(
        std::path::Path::new(
            &std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"),
        ),
        &before,
        supported,
    );
    let capture =
        source_paint_build_recipe::original_command_capture_before(&before, &build.get_archiver());
    build.cargo_debug(capture.enabled());
    source_paint_build_recipe::original_command_capture_begin(&capture);
    build.compile("lumine_source_skia154");
    source_paint_build_recipe::original_command_capture_finish(
        capture,
        &tool_observation(&build.get_compiler()),
        &build.get_archiver(),
        std::path::Path::new(&std::env::var_os("OUT_DIR").expect("Cargo output directory")),
    );
    build.cargo_debug(false);
    source_paint_build_recipe::finish(
        snapshot,
        &tool_observation(&build.get_compiler()),
        std::path::Path::new(&std::env::var_os("OUT_DIR").expect("Cargo output directory")),
    );
    // Preserve the original cc compile/archive execution above. The separate
    // audit actually executes its own commands and must reproduce those bytes;
    // it never claims to have observed the original cc command stream.
    source_paint_build_recipe::audit_after_compile(
        std::path::Path::new(
            &std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"),
        ),
        std::path::Path::new(&std::env::var_os("OUT_DIR").expect("Cargo output directory")),
        cfg!(target_os = "linux") && build.get_compiler().is_like_gnu(),
        || build.get_compiler().to_command(),
        || build.get_archiver(),
    );
}
