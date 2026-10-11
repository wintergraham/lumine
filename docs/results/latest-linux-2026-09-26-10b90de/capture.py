#!/usr/bin/env python3
"""Capture the frozen comparison binaries at the same seeded workload state."""
import os
import json
import hashlib
from pathlib import Path
import subprocess
import time
from run import stop

out = Path(__file__).resolve().parent
binaries = Path(os.environ.get('LUMINE_COMPARISON_BINARIES', '/tmp/lumine-latest-linux-2026-09-26-10b90de-bin'))
display = next(n for n in range(140,180) if not Path(f'/tmp/.X11-unix/X{n}').exists() and not Path(f'/tmp/.X{n}-lock').exists())
env = dict(os.environ, DISPLAY=f':{display}', WINIT_UNIX_BACKEND='x11',
    DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent', LIBGL_ALWAYS_SOFTWARE='1', LP_NUM_THREADS='4',
    VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.json', WGPU_BACKEND='vulkan',
    LD_LIBRARY_PATH=str(Path(json.loads((out / 'preflight.json').read_text())['loader_path']).parent), LUMINE_MODE='idle', LUMINE_SECONDS='3', LUMINE_INITIAL_TICKS='180')
for key in ('WAYLAND_DISPLAY', 'SWAYSOCK', 'I3SOCK', 'X11_SCALE_FACTOR', 'WINIT_X11_SCALE_FACTOR', 'LUMINE_RENDERER', 'LUMINE_EFFECTS'):
    env.pop(key, None)
(out / 'capture-environment.json').write_text(json.dumps({key: env.get(key) for key in (
    'DISPLAY', 'WINIT_UNIX_BACKEND', 'LD_LIBRARY_PATH', 'VK_ICD_FILENAMES',
    'LIBGL_ALWAYS_SOFTWARE', 'WGPU_BACKEND', 'LP_NUM_THREADS', 'LUMINE_MODE',
    'LUMINE_SECONDS', 'LUMINE_INITIAL_TICKS')}, indent=2) + '\n')
x = wm = app = None
try:
    with (out/'capture-xvfb.log').open('w') as log:
        x = subprocess.Popen(['Xvfb',f':{display}','-screen','0','1100x820x24'],stdout=log,stderr=subprocess.STDOUT)
    time.sleep(.5)
    with (out/'capture-openbox.log').open('w') as log:
        wm = subprocess.Popen(['openbox'],env=env,stdout=log,stderr=subprocess.STDOUT)
    time.sleep(.5)
    subprocess.run(['xdotool','mousemove','1090','810'],env=env,check=True)
    for name, binary in [('lumine','component_workload'),('gpui','lumine-compare-gpui'),('quickgui','lumine-compare-quickgui')]:
        with (out/f'capture-{name}.log').open('w') as log:
            app = subprocess.Popen([str(binaries/binary)],env=env,stdout=log,stderr=subprocess.STDOUT)
        time.sleep(1.5)
        (out / f'capture-{name}.maps').write_text(Path(f'/proc/{app.pid}/maps').read_text())
        threads = []
        for task in sorted(Path(f'/proc/{app.pid}/task').iterdir()):
            try:
                threads.append({'tid': int(task.name), 'comm': (task / 'comm').read_text().strip()})
            except FileNotFoundError:
                pass  # A worker may exit while the process remains alive.
        (out / f'capture-{name}-threads.json').write_text(json.dumps(threads, indent=2) + '\n')
        expected = json.loads((out / 'preflight.json').read_text())['binary_sha256'][name]
        assert hashlib.sha256((binaries / binary).read_bytes()).hexdigest() == expected
        windows = subprocess.check_output(['xdotool','search','--onlyvisible','--pid',str(app.pid)],env=env,text=True).split()
        subprocess.run(['import','-window',windows[-1],str(out/f'{name}.png')],env=env,check=True,timeout=10)
        assert app.wait(timeout=15) == 0
        app = None
finally:
    stop(app)
    stop(wm)
    stop(x)
