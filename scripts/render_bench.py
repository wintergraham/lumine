"""Build once, then profile standard lumine workloads or an application's ignored lib test."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
from datetime import datetime, timezone
ROOT = Path(__file__).resolve().parents[1]

def benchmark_record(line):
    # Rust's test harness can prefix the first print with `test name ...`.
    _, marker, payload = line.partition("RENDER_BENCH ")
    return json.loads(payload) if marker else None

def command(args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()

def source_hash(root):
    names = command(["git", "ls-files", "-co", "--exclude-standard"], root).splitlines()
    digest = hashlib.sha256()
    for name in sorted(set(names)):
        path = root / name
        if path.is_file() and (path.suffix in (".rs", ".wgsl", ".json", ".ttf", ".otf", ".svg", ".png", ".jpg", ".manifest", ".c", ".cpp", ".h", ".hpp") or path.name in ("Cargo.toml", "Cargo.lock")):
            digest.update(name.encode())
            digest.update(path.read_bytes())
    return digest.hexdigest()

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--frames", type=int, default=240)
    p.add_argument("--warmup", type=int, default=30)
    p.add_argument("--hz", type=float, default=144)
    p.add_argument("--in-flight", type=int, default=1)
    p.add_argument("--rows", type=int, default=400)
    p.add_argument("--sizes", default="1920x1350@1.5,3840x2160@1.5,5120x2880@2")
    p.add_argument("--only")
    p.add_argument("--profile", choices=("dev", "release"), default="release")
    p.add_argument("--repeats", type=int, default=1)
    p.add_argument("--raw", action="store_true")
    p.add_argument("--trace", type=Path)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--manifest", type=Path, default=ROOT / "Cargo.toml")
    p.add_argument("--target-dir", type=Path)
    p.add_argument("--test", help="Ignored integration lib test; replaces the standard example")
    p.add_argument("--split-shading", choices=("0", "1"))
    p.add_argument("--phase-cache", choices=("0", "1"))
    p.add_argument("--opaque-interiors", choices=("0", "1"))
    args = p.parse_args()
    if args.frames < 1 or args.warmup < 0 or not 0 < args.hz <= 1000 or not 1 <= args.in_flight <= 8 or args.repeats < 1 or not 80 <= args.rows <= 10000:
        p.error("Invalid frame/warmup/refresh/in-flight/repeat/row setting")
    if args.test and (args.in_flight != 1 or args.trace or args.sizes != p.get_default("sizes") or args.rows != p.get_default("rows")):
        p.error("Integration test controls its own sizes/rows/timing; these overrides require the standard example")
    output = args.output.resolve()
    if output.exists() or args.trace and args.trace.exists():
        p.error("Use a new output/trace path; existing evidence is never overwritten")
    if args.trace and args.repeats != 1:
        p.error("Trace one trial at a time")
    output.parent.mkdir(parents=True, exist_ok=True)
    if args.trace:
        args.trace.resolve().parent.mkdir(parents=True, exist_ok=True)
    manifest = args.manifest.resolve()
    project = manifest.parent
    target = (args.target_dir or project / "target").resolve()
    source, renderer_source = source_hash(project), source_hash(ROOT)
    build = ["cargo", "test" if args.test else "build", "--locked", "--manifest-path", str(manifest),
             "--target-dir", str(target), "--profile", args.profile]
    selection = ["--lib", "--no-run"] if args.test else ["-p", "lumine-desktop", "--example", "render_bench"]
    built = command(build + selection + ["--message-format=json"])
    artifacts = [json.loads(line) for line in built.splitlines() if line.startswith("{")]
    if args.test:
        exes = [a["executable"] for a in artifacts if a.get("reason") == "compiler-artifact" and a.get("executable") and a.get("profile", {}).get("test")]
        if len(exes) != 1:
            raise RuntimeError(f"Expected one lib test executable, got {exes}")
        executable = Path(exes[0])
        run_args = [str(executable), args.test, "--ignored", "--nocapture", "--test-threads=1"]
    else:
        executable = target / ("debug" if args.profile == "dev" else "release") / "examples" / ("render_bench.exe" if os.name == "nt" else "render_bench")
        run_args = [str(executable)]
    if source_hash(project) != source or source_hash(ROOT) != renderer_source:
        raise RuntimeError("Source changed during build; rerun for a reproducible binary")
    metadata = dict(started_utc=datetime.now(timezone.utc).isoformat(), platform=platform.platform(),
        rustc=command(["rustc", "--version"]), executable_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
        source_sha256=source, renderer_source_sha256=renderer_source, profile=args.profile,
        warmup=args.warmup, in_flight=args.in_flight, rows=args.rows, split_shading=args.split_shading, phase_cache=args.phase_cache,
        opaque_interiors=args.opaque_interiors,
        build_env={key:value for key,value in os.environ.items() if key in ("RUSTFLAGS", "RUSTC", "WGPU_BACKEND", "WGPU_POWER_PREF") or key.startswith("CARGO_PROFILE_")})
    env = os.environ.copy()
    # Direct execution needs Cargo's runtime library search paths, including
    # native DLLs exported by dependency build scripts (e.g. CEF on Windows).
    profile_dir = target / ("debug" if args.profile == "dev" else "release")
    library_dirs = [str(executable.parent), str(profile_dir / "deps"), str(profile_dir),
                    command(["rustc", "--print", "target-libdir"])]
    for artifact in artifacts:
        for path in artifact.get("linked_paths", []):
            directory = Path(path.split("=", 1)[-1])
            library_dirs.append(str(directory if directory.is_absolute() else project / directory))
    library_key = "PATH" if os.name == "nt" else ("DYLD_FALLBACK_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH")
    env[library_key] = os.pathsep.join(dict.fromkeys(library_dirs + [env.get(library_key, "")]))
    for key in ("ASK_BENCH_OUTPUT", "LUMINE_BENCH_OUTPUT", "LUMINE_BENCH_RAW", "LUMINE_BENCH_ONLY", "LUMINE_BENCH_TRACE", "LUMINE_BENCH_SPLIT_SHADING", "LUMINE_BENCH_PHASE_CACHE", "LUMINE_BENCH_OPAQUE_INTERIORS"):
        env.pop(key, None)
    env.update(LUMINE_BENCH_FRAMES=str(args.frames), LUMINE_BENCH_WARMUP=str(args.warmup),
        LUMINE_BENCH_HZ=str(args.hz), LUMINE_BENCH_IN_FLIGHT=str(args.in_flight), LUMINE_BENCH_ROWS=str(args.rows), LUMINE_BENCH_SIZES=args.sizes)
    if args.raw: env["LUMINE_BENCH_RAW"] = "1"
    if args.only: env["LUMINE_BENCH_ONLY"] = args.only
    if args.trace: env["LUMINE_BENCH_TRACE"] = str(args.trace.resolve())
    if args.split_shading: env["LUMINE_BENCH_SPLIT_SHADING"] = args.split_shading
    if args.phase_cache: env["LUMINE_BENCH_PHASE_CACHE"] = args.phase_cache
    if args.opaque_interiors: env["LUMINE_BENCH_OPAQUE_INTERIORS"] = args.opaque_interiors
    try:
        with output.open("x", encoding="utf-8") as sink:
            for trial in range(1, args.repeats + 1):
                process = subprocess.Popen(run_args, cwd=project, env=env, stdout=subprocess.PIPE, text=True)
                count = 0
                for line in process.stdout:
                    record = benchmark_record(line)
                    if record is None:
                        print(line.rstrip())
                        continue
                    record.update(trial=trial, run=metadata)
                    sink.write(json.dumps(record, separators=(",", ":")) + "\n")
                    sink.flush()
                    count += 1
                    timing = record.get("completed_frame", record.get("cpu_frame", {}))
                    print(f'{record["scene"]}/{record["workload"]} {record["physical_size"]}: p95={timing.get("p95_ms", 0):.3f} ms')
                code = process.wait()
                if code or not count:
                    raise RuntimeError(f"Trial {trial} failed: exit={code}, records={count}; partial evidence: {output}")
    except BaseException:
        if "process" in locals() and process.poll() is None:
            process.terminate()
            process.wait()
        raise
    if source_hash(project) != source or source_hash(ROOT) != renderer_source:
        print("Source changed after building; recorded hashes identify the measured binary.", file=sys.stderr)
    print(f"Results: {output}")

if __name__ == "__main__":
    main()

