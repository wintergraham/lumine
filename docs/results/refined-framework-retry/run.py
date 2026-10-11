#!/usr/bin/env python3
"""Run frozen component/GPUI/QuickGUI release binaries on a private software GPU desktop."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import select
import shutil
import tarfile
import time

ROOT = Path(__file__).resolve().parents[3]

def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binaries', type=Path, default=Path('/tmp/lumine-refined-framework-retry-bin'))
    parser.add_argument('--build-manifest', type=Path, required=True)
    parser.add_argument('--loader', type=Path, default=Path('/tmp/lumine-vulkan-loader-1.4.345/build/loader'))
    parser.add_argument('--preflight-only', action='store_true')
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    def sha(path):
        return hashlib.sha256(path.read_bytes()).hexdigest()
    reference_path = ROOT / 'docs/results/component-comparison/adapters-build-manifest.json'
    references = json.loads(reference_path.read_text())
    built = json.loads(args.build_manifest.read_text())
    for manifest in (references, built):
        for relative, expected in manifest['source_sha256'].items():
            if sha(ROOT / relative) != expected:
                raise RuntimeError('source changed since build: ' + relative)
    names = {'lumine': 'component_workload', 'gpui': 'lumine-compare-gpui', 'quickgui': 'lumine-compare-quickgui'}
    for name, filename in names.items():
        record = (built if name == 'lumine' else references)['binaries'][name]
        if sha(args.binaries / filename) != record['sha256']:
            raise RuntimeError('binary differs from build proof: ' + name)
    loader = (args.loader / 'libvulkan.so.1').resolve(strict=True)
    preflight = dict(binary_sha256={name: sha(args.binaries / filename) for name, filename in names.items()},
        loader_path=str(loader), loader_sha256=sha(loader),
        source_build_proofs_checked=True,
        protocol='three frameworks, four modes, three rotated repeats, 20s total each, 5s excluded warmup')
    (out / 'preflight.json').write_text(json.dumps(preflight, indent=2) + '\n')
    shutil.copyfile(reference_path, out / 'reference-build-manifest.json')
    if args.build_manifest.resolve() != out / 'lumine-build-manifest.json':
        shutil.copyfile(args.build_manifest, out / 'lumine-build-manifest.json')
    if args.preflight_only:
        print('Preflight passes; no GUI processes or samples started')
        return
    if (out / 'current.csv').exists():
        raise RuntimeError('refusing to overwrite an existing measured series')
    processes = subprocess.check_output(['ps', '-eo', 'pid,comm,args'], text=True)
    (out / 'presampling-processes.log').write_text(processes)
    busy = [line for line in processes.splitlines()[1:]
            if len(line.split(None, 2)) > 1 and line.split(None, 2)[1] in ('cargo', 'rustc', 'rust-lld')]
    if busy:
        raise RuntimeError('builds must be paused before sampling: ' + repr(busy))
    source = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock']
    for base in ['crates', 'comparisons', 'scripts', 'assets']:
        source.extend(p for p in (ROOT / base).rglob('*') if p.is_file()
            and 'target' not in p.parts and '__pycache__' not in p.parts
            and p.suffix in ['.rs', '.wgsl', '.toml', '.lock', '.py', '.ttf'])
    source.extend(out.glob('*.py'))
    source.extend(ROOT / relative for relative in built['source_sha256'])
    source = sorted(set(source))
    hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(source)}
    with tarfile.open(out / 'source.tar.gz', 'w:gz') as archive:
        for p in sorted(source):
            archive.add(p, arcname=str(p.relative_to(ROOT)))
    (out / 'source.json').write_text(json.dumps(hashes, indent=2) + '\n')
    env = dict(os.environ, WINIT_UNIX_BACKEND='x11',
        DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent', LIBGL_ALWAYS_SOFTWARE='1',
        VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.json', WGPU_BACKEND='vulkan',
        LD_LIBRARY_PATH=str(args.loader.resolve()))
    for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'SWAYSOCK', 'I3SOCK', 'X11_SCALE_FACTOR',
                'WINIT_X11_SCALE_FACTOR', 'LUMINE_EFFECTS', 'LUMINE_RENDERER'):
        env.pop(key, None)
    desktop = wm = None
    try:
        with (out / 'xvfb.log').open('w') as log:
            desktop = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '1100x820x24'],
                stdout=subprocess.PIPE, stderr=log, text=True)
            if not select.select([desktop.stdout], [], [], 8)[0]:
                raise RuntimeError('Private Xvfb startup timed out')
            env['DISPLAY'] = ':' + desktop.stdout.readline().strip()
        with (out / 'openbox.log').open('w') as log:
            wm = subprocess.Popen(['openbox'], env=env, stdout=log, stderr=subprocess.STDOUT)
        time.sleep(.5)
        subprocess.run(['xdotool', 'mousemove', '1090', '810'], env=env, check=True)
        command = ['python3', str(ROOT / 'scripts/compare.py'),
            '--lumine', str(args.binaries / 'component_workload'),
            '--gpui', str(args.binaries / 'lumine-compare-gpui'),
            '--quickgui', str(args.binaries / 'lumine-compare-quickgui'),
            '--seconds', '20', '--warmup', '5', '--repeats', '3', '--output', str(out / 'current.csv')]
        (out / 'measurement-environment.json').write_text(json.dumps({key: env.get(key) for key in (
            'DISPLAY', 'WAYLAND_DISPLAY', 'LD_LIBRARY_PATH', 'VK_ICD_FILENAMES', 'LIBGL_ALWAYS_SOFTWARE',
            'WINIT_UNIX_BACKEND', 'WGPU_BACKEND', 'LUMINE_RENDERER', 'LUMINE_EFFECTS')}, indent=2) + '\n')
        subprocess.run(command, cwd=ROOT, env=env, check=True)
        subprocess.run(['python3', str(ROOT / 'scripts/summarize_comparison.py'), str(out / 'current.csv'),
            '--output', str(out / 'summary.json')], cwd=ROOT, check=True)
        changed = [p for p, expected in hashes.items() if sha(ROOT / p) != expected]
        if changed:
            raise RuntimeError('sources changed during measurement: ' + repr(changed))
        for name, filename in names.items():
            if sha(args.binaries / filename) != preflight['binary_sha256'][name]:
                raise RuntimeError('measured binary changed: ' + name)
        subprocess.run(['python3', str(ROOT / 'scripts/audit_comparison.py'), str(out / 'current.csv'),
            '--source-manifest', str(out / 'source.json'), '--source-archive', str(out / 'source.tar.gz'),
            '--binaries', str(args.binaries), '--output', str(out / 'audit.json')], cwd=ROOT, check=True)
        if not json.loads((out / 'audit.json').read_text())['near_60hz_workload_updates']:
            raise RuntimeError('model-update parity failed; preserve evidence and investigate before comparison claims')
    finally:
        stop(wm)
        stop(desktop)

if __name__ == '__main__':
    main()
