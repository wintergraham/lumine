#!/usr/bin/env python3
"""Run the three-framework comparison workload on this Mac.

The Linux protocol (docs/results/*/protocol.md), on macOS with the Metal GPU:
three repeats of idle, streaming, scrolling and combined for lumine, GPUI and
QuickGUI, in rotated order. Each trial requests 20 seconds; the first 5 are
excluded, the next 15 sampled every 50 ms by scripts/macsample.swift. Memory
is sampled two ways: resident size, and physical footprint (what Activity
Monitor reports, including GPU memory the process owns on unified memory).
Active trials must deliver 58-61 logical updates per requested second.

`--scene heavy` runs the heavy dashboard instead (lumine_workload::heavy): a
1280x800 window where most content changes every tick, for lumine and GPUI
(QuickGUI's text renderer panics past 32 text batches on it).

Windows open on screen: keep the Mac awake and the windows uncovered (a
covered window may stop presenting). Writes
docs/results/latest-macos[-heavy]-<date>-<commit>.
"""
import argparse, csv, datetime, hashlib, json, os, platform, re, signal, statistics, subprocess, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FRAMEWORKS = ['lumine', 'gpui', 'quickgui']
MODES = ['idle', 'stream', 'scroll', 'both']
REQUESTED, WARMUP, SAMPLED, INTERVAL = 20, 5, 15, 0.05
SCENES = {
    'lab': {
        'lumine': ROOT / 'target/release/examples/component_workload',
        'gpui': ROOT / 'comparisons/gpui/target/release/lumine-compare-gpui',
        'quickgui': ROOT / 'comparisons/quickgui/target/release/lumine-compare-quickgui',
    },
    'heavy': {
        'lumine': ROOT / 'target/release/examples/heavy_workload',
        'gpui': ROOT / 'comparisons/gpui/target/release/heavy',
    },
}
BINARIES = SCENES['lab']


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw).stdout.strip()


def build(sampler):
    example = BINARIES['lumine'].name
    run(['cargo', 'build', '--release', '--locked', '-p', 'lumine-desktop', '--example', example], cwd=ROOT)
    for name in FRAMEWORKS[1:]:
        run(['cargo', 'build', '--release', '--locked'], cwd=ROOT / 'comparisons' / name)
    run(['swiftc', '-O', str(ROOT / 'scripts/macsample.swift'), '-o', str(sampler)])


def environment():
    sysctl = lambda key: run(['sysctl', '-n', key])
    power = run(['pmset', '-g', 'batt']).splitlines()[0]
    return {
        'machine': sysctl('hw.model'),
        'chip': sysctl('machdep.cpu.brand_string'),
        'cores': int(sysctl('hw.ncpu')),
        'memory_bytes': int(sysctl('hw.memsize')),
        'macos': f"{run(['sw_vers', '-productVersion'])} ({run(['sw_vers', '-buildVersion'])})",
        'power': power.split("'")[1] if "'" in power else power,
        'rust': run(['rustc', '--version']),
    }


def trial(framework, mode, repeat, sampler, out):
    log = out / f'{framework}.{mode}.{repeat}.log'
    env = dict(os.environ, LUMINE_MODE=mode, LUMINE_SECONDS=str(REQUESTED))
    with log.open('w') as handle:
        process = subprocess.Popen([str(BINARIES[framework])], stdout=handle, stderr=subprocess.STDOUT, env=env)
    time.sleep(0.5)
    # Bring the window forward: covered windows may stop presenting.
    subprocess.run(['osascript', '-e', f'tell application "System Events" to set frontmost of (first process whose unix id is {process.pid}) to true'], capture_output=True)
    sample = json.loads(run([str(sampler), str(process.pid), str(WARMUP - 0.5), str(SAMPLED), str(INTERVAL)]))
    terminated = False
    try:
        exit_code = process.wait(timeout=REQUESTED + 10)
    except subprocess.TimeoutExpired:
        # macOS apps outlive their last window (QuickGUI closes it at the
        # deadline and reports): end it once it has reported.
        process.send_signal(signal.SIGTERM)
        exit_code, terminated = process.wait(timeout=10), True
    text = log.read_text()
    ticks = re.search(r'"ticks":(\d+)|workload_ticks=(\d+)', text)
    ticks = int(next(g for g in ticks.groups() if g)) if ticks else -1
    row = {'framework': framework, 'mode': mode, 'repeat': repeat, **sample, 'workload_ticks': ticks,
           'exit_code': exit_code, 'terminated_after_report': terminated}
    (out / f'{framework}.{mode}.{repeat}.json').write_text(json.dumps(row, indent=1) + '\n')
    return row


def source_diff_sha256():
    """The uncommitted source: tracked changes and untracked files."""
    paths = ['crates', 'comparisons', 'scripts']
    digest = hashlib.sha256(run(['git', 'diff', 'HEAD', '--', *paths], cwd=ROOT).encode())
    for path in sorted(run(['git', 'ls-files', '--others', '--exclude-standard', '--', *paths], cwd=ROOT).split()):
        digest.update(path.encode() + b'\0' + (ROOT / path).read_bytes())
    return digest.hexdigest()


def main():
    global BINARIES, FRAMEWORKS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--scene', choices=sorted(SCENES), default='lab')
    args = parser.parse_args()
    BINARIES = SCENES[args.scene]
    FRAMEWORKS = list(BINARIES)
    sampler = Path('/tmp/lumine-macsample')
    if not args.skip_build:
        build(sampler)
    commit = run(['git', 'rev-parse', 'HEAD'], cwd=ROOT)
    started = datetime.datetime.now(datetime.timezone.utc)
    scene = '' if args.scene == 'lab' else f'-{args.scene}'
    out = ROOT / 'docs/results' / f'latest-macos{scene}-{started:%Y-%m-%d}-{commit[:7]}'
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for repeat in range(3):
        order = FRAMEWORKS[repeat:] + FRAMEWORKS[:repeat]
        for mode in MODES:
            for framework in order:
                row = trial(framework, mode, repeat, sampler, out)
                print(f"{framework:8} {mode:6} {repeat}: {row['cpu_percent_one_core']:6.2f}% "
                      f"footprint {row['mean_footprint_bytes'] / 2**20:6.1f} MiB rss {row['mean_rss_bytes'] / 2**20:6.1f} MiB "
                      f"ticks {row['workload_ticks']}", flush=True)
                rows.append(row)
                time.sleep(2)
    fields = ['framework', 'mode', 'repeat', 'wall_seconds', 'cpu_seconds', 'cpu_percent_one_core', 'mean_footprint_bytes',
              'peak_footprint_bytes', 'mean_rss_bytes', 'peak_rss_bytes', 'samples', 'workload_ticks', 'exit_code',
              'terminated_after_report']
    with (out / 'current.csv').open('w', newline='') as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)
    metrics = ['cpu_percent_one_core', 'mean_footprint_bytes', 'peak_footprint_bytes', 'mean_rss_bytes', 'peak_rss_bytes', 'workload_ticks', 'wall_seconds']
    groups = []
    for framework in FRAMEWORKS:
        for mode in MODES:
            chosen = [r for r in rows if (r['framework'], r['mode']) == (framework, mode)]
            groups.append({'framework': framework, 'mode': mode, 'repeats': len(chosen), **{
                key: {'median': statistics.median(v), 'min': min(v), 'max': max(v)}
                for key in metrics for v in [[float(r[key]) for r in chosen]]}})
    (out / 'summary.json').write_text(json.dumps({'groups': groups}, indent=1) + '\n')
    active = [r['workload_ticks'] / REQUESTED for r in rows if r['mode'] != 'idle']
    accepted = (all(58 <= t <= 61 for t in active)
                and all(r['workload_ticks'] == 0 for r in rows if r['mode'] == 'idle')
                and all(r['exit_code'] == 0 or r['terminated_after_report'] for r in rows))
    binaries = {name: {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'bytes': path.stat().st_size}
                for name, path in BINARIES.items()}
    audit = {'accepted': accepted, 'trials': len(rows), 'samples': sum(r['samples'] for r in rows),
             'active_ticks_per_requested_second': {'min': min(active), 'max': max(active)},
             'scene': args.scene, 'commit': commit,
             # Uncommitted source changes the binaries were built with, if any.
             'source_dirty': bool(run(['git', 'status', '--porcelain', '--', 'crates', 'comparisons', 'scripts'], cwd=ROOT)),
             'source_diff_sha256': source_diff_sha256(),
             'started_utc': started.isoformat(), 'finished_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
             'environment': environment(), 'binaries': binaries,
             'protocol': {'requested_seconds': REQUESTED, 'warmup_seconds': WARMUP, 'sampled_seconds': SAMPLED,
                          'interval_seconds': INTERVAL, 'repeats': 3, 'frameworks': FRAMEWORKS, 'acceptance_updates_per_second': [58, 61]}}
    (out / 'audit.json').write_text(json.dumps(audit, indent=1) + '\n')
    print(('ACCEPTED' if accepted else 'REJECTED'), out)


if __name__ == '__main__':
    main()
