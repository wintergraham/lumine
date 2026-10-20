//! Build-script-only observation. No dependency changes and no qualification.
//! Hashing uses the same absolute platform SHA256 tool as the source font
//! verifier. Its own executable identity is retained for independent checking.
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_INPUTS: usize = 512;
const ENVIRONMENT: &[&str] = &[
    "HOST",
    "TARGET",
    "OPT_LEVEL",
    "DEBUG",
    "PROFILE",
    "CARGO_CFG_TARGET_FEATURE",
    "CC",
    "CXX",
    "CFLAGS",
    "CXXFLAGS",
    "CPPFLAGS",
    "AR",
    "ARFLAGS",
    "CXXSTDLIB",
    "CRATE_CC_NO_DEFAULTS",
    "CC_SHELL_ESCAPED_FLAGS",
    "CC_KNOWN_WRAPPER_CUSTOM",
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "VCINSTALLDIR",
    "VCToolsInstallDir",
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "CPATH",
    "CPLUS_INCLUDE_PATH",
    "C_INCLUDE_PATH",
    "COMPILER_PATH",
    "GCC_EXEC_PREFIX",
];
const TOOL_ENVIRONMENT: &[&str] = &[
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "SDKROOT",
    "VCToolsInstallDir",
    "VCINSTALLDIR",
];

// A tiny JSON writer avoids changing the native build dependency graph.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Json {
    Null,
    Bool(bool),
    Number(u64),
    Text(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}
fn text(value: impl Into<String>) -> Json {
    Json::Text(value.into())
}
fn object<const N: usize>(fields: [(&str, Json); N]) -> Json {
    Json::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}
fn strings(values: impl IntoIterator<Item = String>) -> Json {
    Json::Array(values.into_iter().map(text).collect())
}
fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
impl Json {
    fn encode(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(v) => v.to_string(),
            Self::Number(v) => v.to_string(),
            Self::Text(v) => quote(v),
            Self::Array(v) => format!(
                "[{}]",
                v.iter().map(Self::encode).collect::<Vec<_>>().join(",")
            ),
            Self::Object(v) => format!(
                "{{{}}}",
                v.iter()
                    .map(|(k, v)| format!("{}:{}", quote(k), v.encode()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}
fn utf8(value: &std::ffi::OsStr) -> Result<String, String> {
    value
        .to_str()
        .filter(|v| v.len() <= 65536)
        .map(str::to_owned)
        .ok_or_else(|| "unbounded-or-non-UTF8-tool-value".into())
}
fn executable(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() || path.components().count() > 1 {
        return fs::canonicalize(path).map_err(|_| "compiler-executable-unavailable".into());
    }
    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(path);
        if candidate.is_file() {
            return fs::canonicalize(candidate)
                .map_err(|_| "compiler-executable-unavailable".into());
        }
    }
    Err("compiler-executable-unavailable".into())
}
#[derive(Clone)]
struct Hasher {
    path: PathBuf,
    args: Vec<&'static str>,
}
impl Hasher {
    fn platform() -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        let value = Self {
            path: PathBuf::from("/usr/bin/sha256sum"),
            args: vec![],
        };
        #[cfg(target_os = "macos")]
        let value = Self {
            path: PathBuf::from("/usr/bin/shasum"),
            args: vec!["-a", "256"],
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err("platform-SHA256-tool-not-observed".into());
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            fs::canonicalize(&value.path).map_err(|_| "platform-SHA256-tool-unavailable")?;
            Ok(value)
        }
    }
    fn hash(&self, path: &Path) -> Result<String, String> {
        let output = Command::new(&self.path)
            .args(&self.args)
            .arg(path)
            .output()
            .map_err(|_| "SHA256-tool-failed")?;
        if !output.status.success() {
            return Err("SHA256-tool-failed".into());
        }
        let output = std::str::from_utf8(&output.stdout).map_err(|_| "invalid-SHA256-output")?;
        let hash = output.split_whitespace().next().unwrap_or("");
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err("invalid-SHA256-output".into());
        }
        Ok(hash.to_owned())
    }
    fn file(&self, path: &Path, label: String) -> Result<Json, String> {
        let meta = fs::symlink_metadata(path).map_err(|_| "recipe-file-unavailable")?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FILE {
            return Err("nonregular-or-unbounded-recipe-file".into());
        }
        Ok(object([
            ("path", text(label)),
            ("sha256", text(self.hash(path)?)),
            ("byteLength", Json::Number(meta.len())),
        ]))
    }
    fn program(&self, path: &Path) -> Result<Json, String> {
        let path = executable(path)?;
        self.file(&path, utf8(path.as_os_str())?)
    }
}

/// Copied from the actual configured cc::Tool and its effective to_command().
/// Never populate this structure from UI/candidate/qualification JSON.
pub struct Tool {
    pub path: PathBuf,
    pub arguments: Vec<OsString>,
    pub command_program: OsString,
    pub command_arguments: Vec<OsString>,
    pub environment: Vec<(OsString, OsString)>,
}
fn tool_json(tool: &Tool, hasher: &Hasher) -> Result<(Json, Vec<String>), String> {
    let mut unknown = Vec::new();
    let mut environment = BTreeMap::new();
    for (key, value) in &tool.environment {
        let key = utf8(key)?;
        if TOOL_ENVIRONMENT.contains(&key.as_str()) {
            environment.insert(key, text(utf8(value)?));
        } else {
            unknown.push(key);
        }
    }
    unknown.sort();
    unknown.dedup();
    let driver = executable(&tool.path)?;
    let version = Command::new(&driver)
        .arg("--version")
        .envs(
            tool.environment
                .iter()
                .filter(|(k, _)| k.to_str().is_some_and(|k| TOOL_ENVIRONMENT.contains(&k)))
                .map(|(k, v)| (k, v)),
        )
        .output()
        .map_err(|_| "compiler-version-unavailable")?;
    if !version.status.success() || version.stdout.len() + version.stderr.len() > 65536 {
        return Err("compiler-version-unavailable".into());
    }
    let value = object([
        ("driver", hasher.program(&driver)?),
        ("toolPath", text(utf8(tool.path.as_os_str())?)),
        (
            "toolArguments",
            strings(
                tool.arguments
                    .iter()
                    .map(|v| utf8(v))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        ),
        (
            "commandProgram",
            hasher.program(Path::new(&tool.command_program))?,
        ),
        (
            "commandArguments",
            strings(
                tool.command_arguments
                    .iter()
                    .map(|v| utf8(v))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        ),
        ("toolEnvironment", Json::Object(environment)),
        ("unreviewedToolEnvironmentKeys", strings(unknown.clone())),
        (
            "version",
            object([
                (
                    "stdout",
                    text(
                        String::from_utf8(version.stdout)
                            .map_err(|_| "non-UTF8-compiler-version")?,
                    ),
                ),
                (
                    "stderr",
                    text(
                        String::from_utf8(version.stderr)
                            .map_err(|_| "non-UTF8-compiler-version")?,
                    ),
                ),
            ]),
        ),
    ]);
    Ok((value, unknown))
}
fn walk(root: &Path, path: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "runtime-inventory-unavailable")?;
    if meta.is_file() {
        paths.push(path.to_owned());
    } else if meta.is_dir() {
        println!("cargo:rerun-if-changed={}", path.display());
        for entry in fs::read_dir(path).map_err(|_| "runtime-inventory-unavailable")? {
            let entry = entry.map_err(|_| "runtime-inventory-unavailable")?;
            walk(root, &entry.path(), paths)?;
        }
    } else {
        return Err("nonregular-runtime-inventory".into());
    }
    if paths.len() > MAX_INPUTS || !path.starts_with(root) {
        return Err("runtime-inventory-budget-exceeded".into());
    }
    Ok(())
}
fn inventory(manifest: &Path, hasher: &Hasher) -> Result<Json, String> {
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or("missing-lumine-root")?;
    let mut paths = Vec::new();
    for path in [
        "crates/lumine/src",
        "crates/lumine-gpu/src",
        "crates/lumine-desktop/src",
        "vendor/skia154-source-paint",
    ] {
        walk(root, &root.join(path), &mut paths)?;
    }
    for path in [
        "Cargo.toml",
        "crates/lumine/Cargo.toml",
        "crates/lumine-gpu/Cargo.toml",
        "crates/lumine-gpu/build.rs",
        "crates/lumine-gpu/source_paint_build_recipe.rs",
        "crates/lumine-desktop/Cargo.toml",
        "scripts/prepare-source-skia154.py",
    ] {
        paths.push(root.join(path));
    }
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_INPUTS {
        return Err("runtime-inventory-budget-exceeded".into());
    }
    let mut files = Vec::new();
    let mut total = 0;
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        total += fs::metadata(&path)
            .map_err(|_| "runtime-input-unavailable")?
            .len();
        if total > 128 * 1024 * 1024 {
            return Err("runtime-inventory-byte-budget-exceeded".into());
        }
        files.push(
            hasher.file(
                &path,
                utf8(
                    path.strip_prefix(root)
                        .map_err(|_| "invalid-runtime-path")?
                        .as_os_str(),
                )?,
            )?,
        );
    }
    Ok(Json::Array(files))
}
fn environment() -> Result<Json, String> {
    let mut keys: Vec<String> = ENVIRONMENT.iter().map(|v| v.to_string()).collect();
    if let Ok(target) = env::var("TARGET") {
        for name in [
            "CC",
            "CXX",
            "CFLAGS",
            "CXXFLAGS",
            "AR",
            "ARFLAGS",
            "CXXSTDLIB",
        ] {
            for suffix in [target.clone(), target.replace('-', "_")] {
                keys.push(format!("{name}_{suffix}"));
            }
            keys.push(format!("HOST_{name}"));
            keys.push(format!("TARGET_{name}"));
        }
    }
    keys.sort();
    keys.dedup();
    let mut values = BTreeMap::new();
    for key in keys {
        println!("cargo:rerun-if-env-changed={key}");
        if let Some(value) = env::var_os(&key) {
            values.insert(key, text(utf8(&value)?));
        }
    }
    Ok(Json::Object(values))
}
fn flags(arguments: &[OsString], supported: bool) -> Json {
    let values: Vec<_> = arguments.iter().filter_map(|v| v.to_str()).collect();
    let fp = values
        .iter()
        .rev()
        .find(|v| v.starts_with("-ffp-contract="));
    let standard = values.iter().rev().find(|v| v.starts_with("-std="));
    let conflict = values
        .iter()
        .any(|v| matches!(*v, "-ffast-math" | "-Ofast"));
    object([
        ("fpContractOffSupported", Json::Bool(supported)),
        (
            "fpContractOffApplied",
            Json::Bool(supported && fp == Some(&"-ffp-contract=off") && !conflict),
        ),
        ("cpp17Applied", Json::Bool(standard == Some(&"-std=c++17"))),
        ("conflictingFastMathFlag", Json::Bool(conflict)),
    ])
}
pub struct Snapshot {
    manifest: PathBuf,
    hasher: Option<Hasher>,
    hasher_identity: Json,
    compiler: Json,
    inputs: Json,
    environment: Json,
    flags: Json,
    problems: Vec<String>,
}
pub fn before(manifest: &Path, tool: &Tool, fp_supported: bool) -> Snapshot {
    let mut value = Snapshot {
        manifest: manifest.to_owned(),
        hasher: None,
        hasher_identity: Json::Null,
        compiler: Json::Null,
        inputs: Json::Null,
        environment: Json::Null,
        flags: flags(&tool.command_arguments, fp_supported),
        problems: Vec::new(),
    };
    let result = (|| -> Result<(), String> {
        let hasher = Hasher::platform()?;
        value.hasher_identity = hasher.program(&hasher.path)?;
        let (compiler, unknown) = tool_json(tool, &hasher)?;
        if !unknown.is_empty() {
            value.problems.push("unreviewed-tool-environment".into());
        }
        value.compiler = compiler;
        value.environment = environment()?;
        value.inputs = inventory(manifest, &hasher)?;
        value.hasher = Some(hasher);
        Ok(())
    })();
    if let Err(error) = result {
        value.problems.push(error);
    }
    if value.flags
        != object([
            ("fpContractOffSupported", Json::Bool(true)),
            ("fpContractOffApplied", Json::Bool(true)),
            ("cpp17Applied", Json::Bool(true)),
            ("conflictingFastMathFlag", Json::Bool(false)),
        ])
    {
        value
            .problems
            .push("required-CXX-flags-not-observed-effective".into());
    }
    value
}
pub fn finish(mut snapshot: Snapshot, tool: &Tool, out_dir: &Path) {
    let mut artifacts = Json::Null;
    let mut hash_tool = Json::Null;
    let result = (|| -> Result<(), String> {
        let hasher = snapshot
            .hasher
            .as_ref()
            .ok_or("SHA256-observation-unavailable")?;
        let (compiler, _) = tool_json(tool, hasher)?;
        if compiler != snapshot.compiler {
            return Err("compiler-changed-during-CXX-build".into());
        }
        if inventory(&snapshot.manifest, hasher)? != snapshot.inputs {
            return Err("runtime-input-changed-during-CXX-build".into());
        }
        if environment()? != snapshot.environment {
            return Err("compiler-environment-changed-during-CXX-build".into());
        }
        let mut objects = Vec::new();
        for entry in fs::read_dir(out_dir).map_err(|_| "CXX-output-directory-unavailable")? {
            let path = entry
                .map_err(|_| "CXX-output-directory-unavailable")?
                .path();
            if path
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|name| {
                    name.ends_with("source_skia154.o") || name.ends_with("source_skia154.obj")
                })
            {
                objects.push(path);
            }
        }
        if objects.len() != 1 {
            return Err("missing-or-ambiguous-source-Skia-object".into());
        }
        let archives = [
            out_dir.join("liblumine_source_skia154.a"),
            out_dir.join("lumine_source_skia154.lib"),
        ];
        let archives: Vec<_> = archives.iter().filter(|path| path.is_file()).collect();
        if archives.len() != 1 {
            return Err("missing-or-ambiguous-source-Skia-archive".into());
        }
        artifacts = object([
            (
                "object",
                hasher.file(&objects[0], utf8(objects[0].as_os_str())?)?,
            ),
            (
                "archive",
                hasher.file(archives[0], utf8(archives[0].as_os_str())?)?,
            ),
        ]);
        hash_tool = hasher.program(&hasher.path)?;
        if hash_tool != snapshot.hasher_identity {
            return Err("SHA256-tool-changed-during-CXX-build".into());
        }
        Ok(())
    })();
    if let Err(error) = result {
        snapshot.problems.push(error);
    }
    let result=object([
        ("version",Json::Number(1)),("kind",text("actual-source-paint-CXX-build-recipe")),("qualification",text("unqualified")),
        ("observationComplete",Json::Bool(snapshot.problems.is_empty())),("problems",strings(snapshot.problems)),
        ("compiler",snapshot.compiler),("buildEnvironment",snapshot.environment),("requiredFlags",snapshot.flags),
        ("runtimeInputs",snapshot.inputs),("artifacts",artifacts),("hashTool",hash_tool),
        ("scope",text("CXX Tool base command and produced source raster object/archive; broad Core/GPU/desktop source inventory, including WGSL and vendor derivation")),
        ("limitations",strings(["No policy authority or source/full-page acceptance".into(),
            "Per-source cc output/source operands are not captured as an executed command log".into(),
            "Actual Cargo resolver/cc package and system headers/compiler subprograms/standard libraries require independent build verification".into()])),
    ]).encode()+"\n";
    let file = out_dir.join("source-paint-build-recipe.json");
    fs::write(&file, result).expect("write source paint build observation");
    let digest = snapshot
        .hasher
        .as_ref()
        .and_then(|hasher| hasher.hash(&file).ok())
        .unwrap_or_default();
    fs::write(out_dir.join("source-paint-build-recipe.sha256"), digest)
        .expect("write source paint build observation digest");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_preserves_controls_quotes_unicode_and_sorted_keys() {
        assert_eq!(quote("a\0\n\t\\\"é"), "\"a\\u0000\\n\\t\\\\\\\"é\"");
        assert_eq!(
            object([("z", Json::Bool(false)), ("a", Json::Null)]).encode(),
            "{\"a\":null,\"z\":false}"
        );
    }
    #[test]
    fn last_effective_flags_and_fast_math_conflicts_are_observed() {
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        let valid = flags(&args(&["-std=c++17", "-ffp-contract=off"]), true);
        for values in [
            &["-std=c++17", "-ffp-contract=off", "-ffp-contract=fast"][..],
            &["-std=c++17", "-ffp-contract=off", "-ffast-math"][..],
            &["-std=c++17", "-std=c++20", "-ffp-contract=off"][..],
        ] {
            assert_ne!(flags(&args(values), true), valid);
        }
        assert_ne!(
            flags(&args(&["-std=c++17", "-ffp-contract=off"]), false),
            valid
        );
    }
    #[test]
    fn absolute_platform_hash_tool_matches_known_sha256() {
        let root =
            env::temp_dir().join(format!("lumine-source-recipe-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("literal $(no shell) bytes");
        fs::write(&file, b"abc").unwrap();
        let hasher = Hasher::platform().unwrap();
        assert_eq!(
            hasher.hash(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let symlink = root.join("link");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&file, &symlink).unwrap();
            assert!(hasher.file(&symlink, "link".into()).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unreviewed_tool_environment_values_are_never_recorded() {
        let tool = Tool {
            path: PathBuf::from("/usr/bin/c++"),
            arguments: vec![],
            command_program: OsString::from("/usr/bin/c++"),
            command_arguments: vec![],
            environment: vec![(
                OsString::from("UNREVIEWED_COMPILER_KEY"),
                OsString::from("private-value-must-not-appear"),
            )],
        };
        let (value, unknown) = tool_json(&tool, &Hasher::platform().unwrap()).unwrap();
        assert_eq!(unknown, vec!["UNREVIEWED_COMPILER_KEY"]);
        assert!(!value.encode().contains("private-value-must-not-appear"));
        assert!(value.encode().contains("unreviewedToolEnvironmentKeys"));
    }
}
