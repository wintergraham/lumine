#!/usr/bin/env python3
"""Run frozen component/GPUI/QuickGUI release binaries on a private software GPU desktop."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
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
    parser.add_argument('--binaries', type=Path, default=Path('/tmp/lumine-component-comparison-bin'))
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    source = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock']
    for base in ['crates', 'comparisons', 'scripts', 'assets']:
        source.extend(p for p in (ROOT / base).rglob('*') if p.is_file()
            and 'target' not in p.parts and '__pycache__' not in p.parts
            and p.suffix in ['.rs', '.toml', '.lock', '.py', '.ttf'])
    source.append(Path(__file__).resolve())
    hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(source)}
    with tarfile.open(out / 'source.tar.gz', 'w:gz') as archive:
        for p in sorted(source):
            archive.add(p, arcname=str(p.relative_to(ROOT)))
    (out / 'source.json').write_text(json.dumps(hashes, indent=2) + '\n')
    display = next(n for n in range(140, 180) if not Path(f'/tmp/.X11-unix/X{n}').exists()
                   and not Path(f'/tmp/.X{n}-lock').exists())
    env = dict(os.environ, DISPLAY=f':{display}', WINIT_UNIX_BACKEND='x11',
        DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent', LIBGL_ALWAYS_SOFTWARE='1',
        VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.json', WGPU_BACKEND='vulkan')
    env.pop('WAYLAND_DISPLAY', None)
    desktop = wm = None
    try:
        with (out / 'xvfb.log').open('w') as log:
            desktop = subprocess.Popen(['Xvfb', f':{display}', '-screen', '0', '1100x820x24'], stdout=log, stderr=subprocess.STDOUT)
        for _ in range(100):
            if desktop.poll() is not None:
                raise RuntimeError('Private Xvfb exited')
            if Path(f'/tmp/.X11-unix/X{display}').exists():
                break
            time.sleep(.05)
        with (out / 'openbox.log').open('w') as log:
            wm = subprocess.Popen(['openbox'], env=env, stdout=log, stderr=subprocess.STDOUT)
        time.sleep(.5)
        subprocess.run(['xdotool', 'mousemove', '1090', '810'], env=env, check=True)
        command = ['python3', str(ROOT / 'scripts/compare.py'),
            '--lumine', str(args.binaries / 'component_workload'),
            '--gpui', str(args.binaries / 'lumine-compare-gpui'),
            '--quickgui', str(args.binaries / 'lumine-compare-quickgui'),
            '--seconds', '10', '--warmup', '3', '--repeats', '2', '--output', str(out / 'current.csv')]
        subprocess.run(command, cwd=ROOT, env=env, check=True)
        subprocess.run(['python3', str(ROOT / 'scripts/summarize_comparison.py'), str(out / 'current.csv'),
            '--output', str(out / 'summary.json')], cwd=ROOT, check=True)
    finally:
        stop(wm)
        stop(desktop)

if __name__ == '__main__':
    main()
