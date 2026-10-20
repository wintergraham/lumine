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

/// Separate, always-unqualified audit. The original cc production execution
/// remains unchanged and is not relabelled as an observed command stream.
pub fn audit_after_compile<C: Fn() -> Command, A: Fn() -> Command>(
    manifest: &Path,
    out: &Path,
    supported: bool,
    compiler: C,
    archiver: A,
) {
    execution_audit::write(manifest, out, supported, compiler, archiver);
}

// A separate audit never claims to observe the original cc process execution.
mod execution_audit {
    use super::*;
    const MAX_DEPENDENCY_BYTES: usize = 1024 * 1024;
    const MAX_HEADERS: usize = 1024;
    const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;
    const MAX_PROGRAM_BYTES: u64 = 128 * 1024 * 1024;

    pub(super) fn reviewed_environment(key: &str, value: &str) -> bool {
        matches!(
            (key, value),
            ("LC_ALL", "C" | "C.UTF-8" | "POSIX") | ("ZERO_AR_DATE", "1")
        )
    }
    pub(super) fn dependency_paths(bytes: &[u8]) -> Result<Vec<String>, String> {
        if bytes.is_empty() || bytes.len() > MAX_DEPENDENCY_BYTES {
            return Err("unbounded-or-empty-dependency-file".into());
        }
        let source = std::str::from_utf8(bytes).map_err(|_| "non-UTF8-dependency-file")?;
        let mut chars = source.chars().peekable();
        let mut after_target = false;
        let mut token = String::new();
        let mut tokens = Vec::new();
        while let Some(c) = chars.next() {
            if c == '\\' {
                let next = chars.next().ok_or("unterminated-dependency-escape")?;
                if next == '\n' {
                    continue;
                }
                if next == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                    continue;
                }
                if !matches!(next, ' ' | '\t' | '#' | ':' | '\\' | '$') {
                    return Err("unsupported-dependency-escape".into());
                }
                token.push(next);
            } else if c == '$' {
                if chars.next() != Some('$') {
                    return Err("unresolved-make-variable-in-dependencies".into());
                }
                token.push('$');
            } else if c == ':' {
                if after_target {
                    return Err("multiple-dependency-rules".into());
                }
                if token.is_empty() {
                    return Err("missing-dependency-target".into());
                }
                token.clear();
                after_target = true;
            } else if c == '#' {
                return Err("unsupported-dependency-comment".into());
            } else if c.is_whitespace() {
                if after_target && !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                } else if !after_target && !token.is_empty() {
                    return Err("multiple-dependency-targets".into());
                }
            } else {
                token.push(c);
            }
        }
        if !after_target {
            return Err("missing-dependency-rule".into());
        }
        if !token.is_empty() {
            tokens.push(token);
        }
        tokens.sort();
        tokens.dedup();
        if tokens.is_empty()
            || tokens.len() > MAX_HEADERS
            || tokens
                .iter()
                .any(|v| v.is_empty() || v.len() > 65536 || v.contains('\0'))
        {
            return Err("dependency-inventory-budget-exceeded".into());
        }
        Ok(tokens)
    }
    // GCC's actual -v child command lines use shell quoting. This parses data
    // only: no shell, expansion, substitution, response-file execution or DSL.
    pub(super) fn shell_words(line: &str) -> Result<Vec<String>, String> {
        if line.len() > 65536 {
            return Err("unbounded-verbose-command".into());
        }
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quote = None;
        let mut escaped = false;
        let mut active = false;
        for c in line.chars() {
            if escaped {
                word.push(c);
                escaped = false;
                active = true;
                continue;
            }
            if quote == Some('\'') {
                if c == '\'' {
                    quote = None;
                } else {
                    word.push(c);
                }
                continue;
            }
            if c == '\\' {
                escaped = true;
                active = true;
                continue;
            }
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                } else {
                    word.push(c);
                }
                continue;
            }
            match c {
                '\'' | '"' => {
                    quote = Some(c);
                    active = true;
                }
                c if c.is_whitespace() => {
                    if active {
                        words.push(std::mem::take(&mut word));
                        active = false;
                    }
                }
                // These are syntax in GCC's display grammar; never interpret
                // ambiguous command chains, expansion or output redirection.
                ';' | '|' | '&' | '<' | '>' | '`' | '$' | '\0' => {
                    return Err("unsupported-verbose-command-syntax".into());
                }
                _ => {
                    word.push(c);
                    active = true;
                }
            }
        }
        if escaped || quote.is_some() {
            return Err("unterminated-verbose-command".into());
        }
        if active {
            words.push(word);
        }
        if words.len() > 4096 {
            return Err("verbose-command-argument-budget-exceeded".into());
        }
        Ok(words)
    }
    fn program_file(hasher: &Hasher, path: &Path) -> Result<Json, String> {
        let path = executable(path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| "audit-program-unavailable")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_PROGRAM_BYTES {
            return Err("audit-program-not-bounded-regular-file".into());
        }
        Ok(object([
            ("path", text(utf8(path.as_os_str())?)),
            ("sha256", text(hasher.hash(&path)?)),
            ("byteLength", Json::Number(metadata.len())),
        ]))
    }
    fn explicit_environment(command: &Command) -> Result<Json, String> {
        let mut result = BTreeMap::new();
        for (key, value) in command.get_envs() {
            let key = utf8(key)?;
            let value = value.ok_or("audit-explicit-environment-removal-unreviewed")?;
            let value = utf8(value)?;
            if !reviewed_environment(&key, &value) {
                // Never format the rejected value or complete Command Debug.
                return Err("audit-explicit-environment-unreviewed".into());
            }
            result.insert(key, text(value));
        }
        Ok(Json::Object(result))
    }
    fn command_identity(command: &Command, cwd: &Path, hasher: &Hasher) -> Result<Json, String> {
        let arguments = command
            .get_args()
            .map(utf8)
            .collect::<Result<Vec<_>, _>>()?;
        if arguments.len() > 4096 || arguments.iter().map(String::len).sum::<usize>() > 256 * 1024 {
            return Err("audit-argument-budget-exceeded".into());
        }
        Ok(object([
            (
                "program",
                program_file(hasher, Path::new(command.get_program()))?,
            ),
            ("requestedProgram", text(utf8(command.get_program())?)),
            ("arguments", strings(arguments)),
            ("cwd", text(utf8(cwd.as_os_str())?)),
            ("explicitEnvironment", explicit_environment(command)?),
            ("inheritedEnvironmentDumped", Json::Bool(false)),
        ]))
    }
    struct Run {
        record: Json,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        success: bool,
    }
    fn execute(
        mut command: Command,
        cwd: &Path,
        out: &Path,
        role: &str,
        hasher: &Hasher,
        deadline: std::time::Instant,
    ) -> Result<Run, String> {
        use std::{
            io::Read,
            process::Stdio,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            time::{Duration, Instant},
        };
        let started = Instant::now();
        if started >= deadline {
            return Err("audit-total-deadline-exceeded".into());
        }
        command.current_dir(cwd);
        let before = command_identity(&command, cwd, hasher)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|_| "audit-command-spawn-failed")?;
        let excessive = Arc::new(AtomicBool::new(false));
        let capture = |mut pipe: Box<dyn Read + Send>, flag: Arc<AtomicBool>| {
            std::thread::spawn(move || {
                let mut result = Vec::new();
                let mut buffer = [0; 8192];
                loop {
                    match pipe.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            let remaining = MAX_LOG_BYTES.saturating_sub(result.len());
                            result.extend_from_slice(&buffer[..count.min(remaining)]);
                            if count > remaining {
                                flag.store(true, Ordering::Relaxed);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => {
                            flag.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                result
            })
        };
        let stdout = capture(
            Box::new(child.stdout.take().ok_or("audit-stdout-unavailable")?),
            excessive.clone(),
        );
        let stderr = capture(
            Box::new(child.stderr.take().ok_or("audit-stderr-unavailable")?),
            excessive.clone(),
        );
        let mut terminated = false;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|_| "audit-command-wait-failed")? {
                break status;
            }
            if Instant::now() >= deadline
                || started.elapsed() > Duration::from_secs(60)
                || excessive.load(Ordering::Relaxed)
            {
                terminated = true;
                #[cfg(target_os = "linux")]
                unsafe {
                    unsafe extern "C" {
                        fn kill(pid: i32, signal: i32) -> i32;
                    }
                    // The child was placed in its own process group above;
                    // terminate compiler children as well as the driver.
                    kill(-(child.id() as i32), 9);
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = child.kill();
                }
                break child
                    .wait()
                    .map_err(|_| "audit-terminated-command-wait-failed")?;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = stdout.join().map_err(|_| "audit-stdout-reader-failed")?;
        let stderr = stderr.join().map_err(|_| "audit-stderr-reader-failed")?;
        let output = |suffix: &str, bytes: &[u8]| -> Result<Json, String> {
            let file = out.join(format!("{role}.{suffix}"));
            fs::write(&file, bytes).map_err(|_| "audit-output-write-failed")?;
            // Empty process streams are genuine and retained with SHA256 too.
            Ok(object([
                ("path", text(utf8(file.as_os_str())?)),
                ("sha256", text(hasher.hash(&file)?)),
                ("byteLength", Json::Number(bytes.len() as u64)),
            ]))
        };
        let after = command_identity(&command, cwd, hasher)?;
        let success = status.success()
            && !terminated
            && !excessive.load(Ordering::Relaxed)
            && before == after;
        let code = status
            .code()
            .and_then(|value| u64::try_from(value).ok())
            .map_or(Json::Null, Json::Number);
        let record = object([
            ("role", text(role)),
            ("kind", text("actual-audit-command-execution")),
            ("commandBefore", before),
            ("commandAfter", after),
            ("exitCode", code),
            ("success", Json::Bool(success)),
            ("terminatedByBound", Json::Bool(terminated)),
            (
                "outputTruncated",
                Json::Bool(excessive.load(Ordering::Relaxed)),
            ),
            (
                "durationMs",
                Json::Number(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            ),
            ("stdout", output("stdout", &stdout)?),
            ("stderr", output("stderr", &stderr)?),
        ]);
        Ok(Run {
            record,
            stdout,
            stderr,
            success,
        })
    }
    fn header_inventory(
        depfile: &Path,
        cwd: &Path,
        hasher: &Hasher,
    ) -> Result<(Json, Vec<PathBuf>), String> {
        let bytes = fs::read(depfile).map_err(|_| "audit-dependency-file-unavailable")?;
        let tokens = dependency_paths(&bytes)?;
        let mut result = Vec::new();
        let mut paths = Vec::new();
        let mut total = 0;
        for token in tokens {
            let original = PathBuf::from(&token);
            let path = fs::canonicalize(if original.is_absolute() {
                original
            } else {
                cwd.join(original)
            })
            .map_err(|_| "audit-reported-header-unavailable")?;
            total += fs::metadata(&path)
                .map_err(|_| "audit-header-unavailable")?
                .len();
            if total > 128 * 1024 * 1024 {
                return Err("audit-header-byte-budget-exceeded".into());
            }
            result.push(object([
                ("reportedPath", text(token)),
                ("file", hasher.file(&path, utf8(path.as_os_str())?)?),
            ]));
            paths.push(path);
        }
        paths.sort();
        paths.dedup();
        Ok((Json::Array(result), paths))
    }
    fn verbose_programs(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<String>)>, String> {
        let source = std::str::from_utf8(bytes).map_err(|_| "audit-verbose-output-not-UTF8")?;
        let mut result = Vec::new();
        for line in source
            .lines()
            .filter(|line| line.starts_with(' ') || line.starts_with('\t'))
        {
            let words = shell_words(line)?;
            let Some(program) = words.first() else {
                continue;
            };
            // Directory-only include-search diagnostics are not child commands.
            if let Ok(path) = executable(Path::new(program)) {
                if fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                    result.push((path, words));
                }
            }
        }
        Ok(result)
    }
    struct Audit {
        commands: Vec<Json>,
        configured_tools: Json,
        runtime_inputs: Json,
        headers: Json,
        helpers: Json,
        object_identity: Json,
        archive_identity: Json,
    }
    impl Audit {
        fn command(
            &mut self,
            command: Command,
            cwd: &Path,
            out: &Path,
            role: &str,
            hasher: &Hasher,
            deadline: std::time::Instant,
        ) -> Result<Run, String> {
            let result = execute(command, cwd, out, role, hasher, deadline)?;
            self.commands.push(result.record.clone());
            if !result.success {
                return Err("audit-executed-command-failed-or-bounded".into());
            }
            Ok(result)
        }
    }
    fn same_file_bytes(left: &Json, right: &Json) -> bool {
        match (left, right) {
            (Json::Object(a), Json::Object(b)) => {
                a.get("sha256") == b.get("sha256") && a.get("byteLength") == b.get("byteLength")
            }
            _ => false,
        }
    }
    fn production_artifacts(out: &Path) -> Result<(PathBuf, PathBuf), String> {
        let mut objects = Vec::new();
        for entry in fs::read_dir(out).map_err(|_| "audit-production-directory-unavailable")? {
            let path = entry
                .map_err(|_| "audit-production-directory-unavailable")?
                .path();
            if path
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(|name| name.ends_with("source_skia154.o"))
            {
                objects.push(path);
            }
        }
        if objects.len() != 1 {
            return Err("audit-production-object-ambiguous".into());
        }
        let archive = out.join("liblumine_source_skia154.a");
        if !archive.is_file() {
            return Err("audit-production-archive-unavailable".into());
        }
        Ok((objects.remove(0), archive))
    }
    impl Audit {
        fn run<C: Fn() -> Command, A: Fn() -> Command>(
            &mut self,
            manifest: &Path,
            out: &Path,
            compiler: C,
            archiver: A,
            hasher: &Hasher,
        ) -> Result<(), String> {
            use std::time::{Duration, Instant};
            let deadline = Instant::now() + Duration::from_secs(120);
            let cwd =
                fs::canonicalize(manifest).map_err(|_| "audit-source-directory-unavailable")?;
            let directory = out.join(format!(
                "source-paint-reproducibility-audit-{}",
                std::process::id()
            ));
            fs::create_dir(&directory).map_err(|_| "audit-fresh-directory-unavailable")?;
            let (production_object, production_archive) = production_artifacts(out)?;
            let object_before =
                hasher.file(&production_object, utf8(production_object.as_os_str())?)?;
            let archive_before =
                hasher.file(&production_archive, utf8(production_archive.as_os_str())?)?;
            let audit_object_path = directory.join(
                production_object
                    .file_name()
                    .ok_or("audit-object-basename-unavailable")?,
            );
            let archive = directory.join(
                production_archive
                    .file_name()
                    .ok_or("audit-archive-basename-unavailable")?,
            );
            let pre_dep = directory.join("headers.preflight.d");
            let actual_dep = directory.join("headers.actual.d");
            let recipe = out.join("source-paint-build-recipe.json");
            let recipe_before = hasher.file(&recipe, utf8(recipe.as_os_str())?)?;
            let runtime_before = inventory(&cwd, hasher)?;
            // The known v1 writer emits this canonical value verbatim. Bind
            // the audit's live inventory to the immutable production receipt;
            // independent readers still parse and verify both complete JSONs.
            let production_json =
                fs::read_to_string(&recipe).map_err(|_| "audit-production-recipe-unavailable")?;
            if !production_json.contains(&format!("\"runtimeInputs\":{}", runtime_before.encode()))
            {
                return Err("audit-runtime-inventory-differs-from-production-recipe".into());
            }
            let hash_tool_before = hasher.program(&hasher.path)?;
            if !production_json.contains(&format!("\"hashTool\":{}", hash_tool_before.encode())) {
                return Err("audit-hash-tool-differs-from-production-recipe".into());
            }
            let compiler_before = command_identity(&compiler(), &cwd, hasher)?;
            let archiver_before = command_identity(&archiver(), &cwd, hasher)?;
            // This query is candidate discovery, not evidence of execution of
            // the returned program. Only the later actual -v invocation can
            // establish compiler-reported use of the same prehashed bytes.
            let mut discovery = compiler();
            discovery.arg("-print-prog-name=as");
            let discovery = self.command(
                discovery,
                &cwd,
                &directory,
                "assembler-discovery",
                hasher,
                deadline,
            )?;
            let assembler = std::str::from_utf8(&discovery.stdout)
                .map_err(|_| "audit-assembler-discovery-not-UTF8")?
                .trim();
            if assembler.is_empty() || assembler.lines().count() != 1 {
                return Err("audit-assembler-discovery-ambiguous".into());
            }
            let assembler = executable(Path::new(assembler))?;
            let mut preflight = compiler();
            preflight
                .args(["-v", "-M", "-MF"])
                .arg(&pre_dep)
                .arg("-MT")
                .arg("source-paint-audit-object")
                .arg("src/source_skia154.cpp");
            let preflight = self.command(
                preflight,
                &cwd,
                &directory,
                "header-preflight",
                hasher,
                deadline,
            )?;
            let (headers_before, header_paths) = header_inventory(&pre_dep, &cwd, hasher)?;
            let reported_preflight = verbose_programs(&preflight.stderr)?;
            if reported_preflight.is_empty() {
                return Err("audit-preflight-child-commands-unavailable".into());
            }
            let mut helpers = reported_preflight
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            helpers.push(assembler.clone());
            helpers.sort();
            helpers.dedup();
            let helper_before = helpers
                .iter()
                .map(|path| program_file(hasher, path))
                .collect::<Result<Vec<_>, _>>()?;
            let mut compile = compiler();
            compile
                .args(["-MD", "-MF"])
                .arg(&actual_dep)
                .arg("-v")
                .arg("-o")
                .arg(&audit_object_path)
                .arg("-c")
                .arg("src/source_skia154.cpp");
            let compiled = self.command(compile, &cwd, &directory, "compile", hasher, deadline)?;
            let (headers_after, actual_paths) = header_inventory(&actual_dep, &cwd, hasher)?;
            let (preflight_after, preflight_paths_after) =
                header_inventory(&pre_dep, &cwd, hasher)?;
            let headers_match = header_paths == actual_paths
                && header_paths == preflight_paths_after
                && headers_before == preflight_after;
            self.headers = object([
                (
                    "kind",
                    text("compiler-reported-M-preflight-and-actual-MD-dependencies"),
                ),
                (
                    "preflightDepfile",
                    hasher.file(&pre_dep, utf8(pre_dep.as_os_str())?)?,
                ),
                (
                    "actualCompilationDepfile",
                    hasher.file(&actual_dep, utf8(actual_dep.as_os_str())?)?,
                ),
                ("before", headers_before),
                ("after", headers_after),
                ("preflightRecheckedAfter", preflight_after),
                ("canonicalSetsAndBytesMatch", Json::Bool(headers_match)),
                ("kernelFileReadTraceObserved", Json::Bool(false)),
            ]);
            if !headers_match {
                return Err("audit-actual-dependencies-or-header-bytes-differ".into());
            }
            let reported_compile = verbose_programs(&compiled.stderr)?;
            let observed_paths = reported_compile
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            let matched = !reported_compile.is_empty()
                && observed_paths.contains(&assembler)
                && helpers.iter().all(|path| observed_paths.contains(path))
                && observed_paths.iter().all(|path| helpers.contains(path));
            let helper_after = helpers
                .iter()
                .map(|path| program_file(hasher, path))
                .collect::<Result<Vec<_>, _>>()?;
            self.helpers = object([
                (
                    "evidenceKind",
                    text("actual-audit-GCC-v-compiler-reported-child-invocations"),
                ),
                ("prehashedPrograms", Json::Array(helper_before.clone())),
                ("posthashedPrograms", Json::Array(helper_after.clone())),
                (
                    "actualAuditCompileReportedInvocations",
                    Json::Array(
                        reported_compile
                            .into_iter()
                            .map(|(path, args)| {
                                object([
                                    ("resolvedProgram", text(path.to_string_lossy())),
                                    ("reportedArguments", strings(args)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("discoveredAndReportedProgramsMatch", Json::Bool(matched)),
                ("kernelExecTraceObserved", Json::Bool(false)),
            ]);
            if !matched || helper_before != helper_after {
                return Err("audit-helper-use-or-bytes-unresolved".into());
            }
            let audit_object =
                hasher.file(&audit_object_path, utf8(audit_object_path.as_os_str())?)?;
            let object_matches = same_file_bytes(&object_before, &audit_object);
            self.object_identity = object([
                ("actualProduction", object_before.clone()),
                ("actualAudit", audit_object),
                ("byteIdentical", Json::Bool(object_matches)),
            ]);
            if !object_matches {
                return Err("audit-object-not-byte-identical-to-production".into());
            }
            let mut create = archiver();
            create
                .env("ZERO_AR_DATE", "1")
                .arg("cqD")
                .arg(&archive)
                .arg(&audit_object_path);
            self.command(create, &cwd, &directory, "archive-create", hasher, deadline)?;
            let mut index = archiver();
            index.arg("sD").arg(&archive);
            self.command(index, &cwd, &directory, "archive-index", hasher, deadline)?;
            let audit_archive = hasher.file(&archive, utf8(archive.as_os_str())?)?;
            let archive_matches = same_file_bytes(&archive_before, &audit_archive);
            self.archive_identity = object([
                ("actualProduction", archive_before.clone()),
                ("actualAudit", audit_archive),
                ("byteIdentical", Json::Bool(archive_matches)),
            ]);
            if !archive_matches {
                return Err("audit-archive-not-byte-identical-to-production".into());
            }
            let hash_tool_after = hasher.program(&hasher.path)?;
            let compiler_after = command_identity(&compiler(), &cwd, hasher)?;
            let archiver_after = command_identity(&archiver(), &cwd, hasher)?;
            self.configured_tools = object([
                (
                    "kind",
                    text("configured-command-identities-before-and-after-audit"),
                ),
                ("hashToolBefore", hash_tool_before.clone()),
                ("hashToolAfter", hash_tool_after.clone()),
                ("compilerBefore", compiler_before.clone()),
                ("compilerAfter", compiler_after.clone()),
                ("archiverBefore", archiver_before.clone()),
                ("archiverAfter", archiver_after.clone()),
            ]);
            let runtime_after = inventory(&cwd, hasher)?;
            self.runtime_inputs = object([
                ("before", runtime_before.clone()),
                ("after", runtime_after.clone()),
            ]);
            if hash_tool_before != hash_tool_after
                || compiler_before != compiler_after
                || archiver_before != archiver_after
            {
                return Err("audit-configured-tool-changed-during-audit".into());
            }
            if hasher.file(&production_object, utf8(production_object.as_os_str())?)?
                != object_before
                || hasher.file(&production_archive, utf8(production_archive.as_os_str())?)?
                    != archive_before
                || hasher.file(&recipe, utf8(recipe.as_os_str())?)? != recipe_before
                || runtime_after != runtime_before
            {
                return Err("audit-production-artifacts-or-runtime-changed".into());
            }
            Ok(())
        }
    }
    pub(super) fn write<C: Fn() -> Command, A: Fn() -> Command>(
        manifest: &Path,
        out: &Path,
        supported: bool,
        compiler: C,
        archiver: A,
    ) {
        let mut audit = Audit {
            commands: Vec::new(),
            configured_tools: Json::Null,
            runtime_inputs: Json::Null,
            headers: Json::Null,
            helpers: Json::Null,
            object_identity: Json::Null,
            archive_identity: Json::Null,
        };
        let hasher = Hasher::platform().ok();
        let result = if supported {
            hasher
                .as_ref()
                .ok_or("audit-hash-tool-unavailable".into())
                .and_then(|hasher| audit.run(manifest, out, compiler, archiver, hasher))
        } else {
            Err("audit-platform-or-tool-family-unsupported".into())
        };
        let complete = result.is_ok();
        let problems = result.err().into_iter().collect::<Vec<_>>();
        let production_recipe = hasher
            .as_ref()
            .and_then(|hasher| {
                hasher
                    .file(
                        &out.join("source-paint-build-recipe.json"),
                        out.join("source-paint-build-recipe.json")
                            .to_string_lossy()
                            .into_owned(),
                    )
                    .ok()
            })
            .unwrap_or(Json::Null);
        let receipt = object([
            ("version", Json::Number(1)), ("kind", text("actual-source-paint-CXX-reproducibility-audit")),
            ("qualification", text("unqualified")), ("acceptance", Json::Bool(false)), ("activationQualified", Json::Bool(false)),
            ("reproducibilityObserved", Json::Bool(complete)), ("executionClosureComplete", Json::Bool(false)),
            ("originalProductionCommandsObserved", Json::Bool(false)),
            ("problems", strings(problems)), ("productionRecipe", production_recipe),
            ("auditCommands", Json::Array(audit.commands)),
            ("configuredTools", audit.configured_tools),
            ("runtimeInputs", audit.runtime_inputs), ("headers", audit.headers), ("helpers", audit.helpers),
            ("objectIdentity", audit.object_identity), ("archiveIdentity", audit.archive_identity),
            ("limits", object([("totalSeconds", Json::Number(120)), ("commandSeconds", Json::Number(60)), ("streamBytes", Json::Number(MAX_LOG_BYTES as u64)), ("headerFiles", Json::Number(MAX_HEADERS as u64)), ("headerTotalBytes", Json::Number(128 * 1024 * 1024)), ("programFileBytes", Json::Number(MAX_PROGRAM_BYTES))])),
            ("limitations", strings([
                "Audit executions are distinct from original cc production commands; byte identity is reproducibility evidence only".into(),
                "GCC verbose invocation lines and -M/-MD are compiler-reported evidence, not kernel exec/file-read tracing".into(),
                "Compiler shared libraries, archiver plugins, final native linker and loaded C++ standard library closure remain unresolved".into(),
                "Inherited environment is preserved and never dumped; explicit audit command environment accepts reviewed nonsecret literals only".into(),
                "No source paint capability, qualification, activation, or full-page acceptance".into(),
            ])),
        ]).encode() + "\n";
        let path = out.join("source-paint-execution-audit.json");
        fs::write(&path, receipt).expect("write source paint execution audit");
        let digest = hasher
            .and_then(|hasher| hasher.hash(&path).ok())
            .unwrap_or_default();
        fs::write(out.join("source-paint-execution-audit.sha256"), digest)
            .expect("write source paint audit digest");
    }
    #[cfg(test)]
    #[test]
    fn actual_runner_caps_streams_and_terminates_its_own_deadline_group() {
        use std::time::{Duration, Instant};
        let root =
            env::temp_dir().join(format!("lumine-audit-runner-limits-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let hasher = Hasher::platform().unwrap();
        let mut command = Command::new("/usr/bin/head");
        command.args(["-c", &(MAX_LOG_BYTES + 16384).to_string(), "/dev/zero"]);
        let run = execute(
            command,
            &root,
            &root,
            "over-stream-limit",
            &hasher,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert!(!run.success);
        assert_eq!(run.stdout.len(), MAX_LOG_BYTES);
        assert!(run.record.encode().contains("\"outputTruncated\":true"));
        let mut command = Command::new("/usr/bin/sleep");
        command.arg("2");
        let run = execute(
            command,
            &root,
            &root,
            "over-deadline",
            &hasher,
            Instant::now() + Duration::from_millis(20),
        )
        .unwrap();
        assert!(!run.success);
        assert!(run.record.encode().contains("\"terminatedByBound\":true"));
        fs::remove_dir_all(root).unwrap();
    }
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
    #[test]
    fn audit_dependency_rules_preserve_actual_escaped_paths_and_reject_ambiguity() {
        let paths = execution_audit::dependency_paths(b"object.o: source.cpp /usr/include/a\\ b.h \\\n /headers/hash\\#.h /headers/colon\\:.h /headers/dollar$$.h\n").unwrap();
        assert_eq!(
            paths,
            vec![
                "/headers/colon:.h",
                "/headers/dollar$.h",
                "/headers/hash#.h",
                "/usr/include/a b.h",
                "source.cpp"
            ]
        );
        for bad in [
            b"".as_slice(),
            b"header.h",
            b"x.o: $(UNREVIEWED)",
            b"x.o: a.h\ny.o: b.h",
            b"x.o: a.h # comment",
            b"x.o: dangling\\",
            b"x.o: a\\q.h",
        ] {
            assert!(execution_audit::dependency_paths(bad).is_err());
        }
    }
    #[test]
    fn audit_verbose_command_parser_is_data_only_and_rejects_expansion() {
        assert_eq!(
            execution_audit::shell_words(
                " /actual/cc1plus -quiet 'source with spaces.cpp' \"double quoted\" a\\ b"
            )
            .unwrap(),
            vec![
                "/actual/cc1plus",
                "-quiet",
                "source with spaces.cpp",
                "double quoted",
                "a b"
            ]
        );
        for bad in [
            "as ; other",
            "as $(secret)",
            "as `command`",
            "as > output",
            "as 'unterminated",
            "as dangling\\",
        ] {
            assert!(execution_audit::shell_words(bad).is_err());
        }
    }
    #[test]
    fn audit_environment_records_only_reviewed_nonsecret_literals() {
        for value in ["C", "C.UTF-8", "POSIX"] {
            assert!(execution_audit::reviewed_environment("LC_ALL", value));
        }
        assert!(execution_audit::reviewed_environment("ZERO_AR_DATE", "1"));
        assert!(!execution_audit::reviewed_environment(
            "LC_ALL",
            "private-unreviewed-value"
        ));
        assert!(!execution_audit::reviewed_environment("UNREVIEWED", "C"));
    }
    #[test]
    #[ignore = "owner serial bounded runner supplies installed GNU C++ and archiver; no Cargo/GPU"]
    fn audit_actual_cpp_reproduction_and_changed_header_fail_closed() {
        if !cfg!(target_os = "linux") {
            return;
        }
        use std::cell::Cell;
        let root = env::temp_dir().join(format!("lumine-cxx-audit-fixture-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let manifest = root.join("crates/lumine-gpu");
        for directory in [
            "crates/lumine/src",
            "crates/lumine-gpu/src",
            "crates/lumine-desktop/src",
            "vendor/skia154-source-paint",
            "scripts",
        ] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        for file in [
            "Cargo.toml",
            "crates/lumine/Cargo.toml",
            "crates/lumine-gpu/Cargo.toml",
            "crates/lumine-gpu/build.rs",
            "crates/lumine-gpu/source_paint_build_recipe.rs",
            "crates/lumine-desktop/Cargo.toml",
            "scripts/prepare-source-skia154.py",
        ] {
            fs::write(root.join(file), "// synthetic unit-test inventory only\n").unwrap();
        }
        let source = manifest.join("src/source_skia154.cpp");
        let header = manifest.join("src/observed.h");
        fs::write(&header, "#define AUDIT_VALUE 7\n").unwrap();
        fs::write(&source, "#include \"observed.h\"\n#include <vector>\nint audit_value() { std::vector<int> v{AUDIT_VALUE}; return v[0]; }\n").unwrap();
        let compiler = || {
            let mut command = Command::new("/usr/bin/c++");
            command
                .args(["-std=c++17", "-O1", "-ffp-contract=off"])
                .env("LC_ALL", "C");
            command
        };
        let archiver = || Command::new("/usr/bin/ar");
        let out = root.join("out");
        fs::create_dir(&out).unwrap();
        let object = out.join("unit-source_skia154.o");
        let archive = out.join("liblumine_source_skia154.a");
        let mut production = compiler();
        assert!(
            production
                .current_dir(&manifest)
                .arg("-o")
                .arg(&object)
                .arg("-c")
                .arg("src/source_skia154.cpp")
                .status()
                .unwrap()
                .success()
        );
        assert!(
            archiver()
                .arg("cqD")
                .arg(&archive)
                .arg(&object)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            archiver()
                .arg("sD")
                .arg(&archive)
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            out.join("source-paint-build-recipe.json"),
            super::object([
                ("kind", text("synthetic-unit-test-production-artifacts")),
                ("qualification", text("unqualified")),
                ("hashTool", {
                    let hasher = Hasher::platform().unwrap();
                    hasher.program(&hasher.path).unwrap()
                }),
                (
                    "runtimeInputs",
                    inventory(&manifest, &Hasher::platform().unwrap()).unwrap(),
                ),
            ])
            .encode(),
        )
        .unwrap();
        audit_after_compile(&manifest, &out, true, compiler, archiver);
        let receipt = fs::read_to_string(out.join("source-paint-execution-audit.json")).unwrap();
        assert!(
            receipt.contains("\"reproducibilityObserved\":true"),
            "{}",
            receipt
        );
        assert!(receipt.contains("\"executionClosureComplete\":false"));
        assert!(receipt.contains("\"originalProductionCommandsObserved\":false"));
        assert!(receipt.contains("\"LC_ALL\":\"C\""));
        assert!(receipt.contains("\"canonicalSetsAndBytesMatch\":true"));
        assert!(receipt.contains("\"byteIdentical\":true"));
        // A separate fresh audit transaction uses the same actual production
        // artifacts. The compiler factory mutates a consumed header between
        // preflight snapshot and actual audit compile, and must be rejected.
        let changed = root.join("changed");
        fs::create_dir(&changed).unwrap();
        fs::copy(&object, changed.join(object.file_name().unwrap())).unwrap();
        fs::copy(&archive, changed.join(archive.file_name().unwrap())).unwrap();
        fs::copy(
            out.join("source-paint-build-recipe.json"),
            changed.join("source-paint-build-recipe.json"),
        )
        .unwrap();
        let calls = Cell::new(0);
        audit_after_compile(
            &manifest,
            &changed,
            true,
            || {
                let index = calls.get();
                calls.set(index + 1);
                if index == 3 {
                    fs::write(&header, "#define AUDIT_VALUE 9\n").unwrap();
                }
                compiler()
            },
            archiver,
        );
        let rejected =
            fs::read_to_string(changed.join("source-paint-execution-audit.json")).unwrap();
        assert!(rejected.contains("\"reproducibilityObserved\":false"));
        assert!(rejected.contains("audit-actual-dependencies-or-header-bytes-differ"));
        // Restore the independent fixture state before testing environment
        // rejection; the prior header mutation must not mask this boundary.
        fs::write(&header, "#define AUDIT_VALUE 7\n").unwrap();
        let secret = root.join("secret");
        fs::create_dir(&secret).unwrap();
        fs::copy(&object, secret.join(object.file_name().unwrap())).unwrap();
        fs::copy(&archive, secret.join(archive.file_name().unwrap())).unwrap();
        fs::copy(
            out.join("source-paint-build-recipe.json"),
            secret.join("source-paint-build-recipe.json"),
        )
        .unwrap();
        audit_after_compile(
            &manifest,
            &secret,
            true,
            || {
                let mut command = compiler();
                command.env("UNREVIEWED_ENVIRONMENT", "private-must-never-be-recorded");
                command
            },
            archiver,
        );
        let rejected =
            fs::read_to_string(secret.join("source-paint-execution-audit.json")).unwrap();
        assert!(rejected.contains("\"reproducibilityObserved\":false"));
        assert!(rejected.contains("audit-explicit-environment-unreviewed"));
        assert!(!rejected.contains("private-must-never-be-recorded"));
        fs::remove_dir_all(root).unwrap();
    }
}
