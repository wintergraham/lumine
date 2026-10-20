fn main() {
    println!("cargo:rerun-if-changed=src/source_skia154.cpp");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .flag_if_supported("-ffp-contract=off")
        .file("src/source_skia154.cpp")
        .compile("lumine_source_skia154");
}
