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
    build.compile("lumine_source_skia154");
    source_paint_build_recipe::finish(
        snapshot,
        &tool_observation(&build.get_compiler()),
        std::path::Path::new(&std::env::var_os("OUT_DIR").expect("Cargo output directory")),
    );
}
