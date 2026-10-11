#!/usr/bin/env python3
"""Same-binary backend-selection ablation; snapshots are OS mappings, not allocator attribution."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

out = Path(__file__).resolve().parent / 'backend-ablation'
out.mkdir(exist_ok=True)
bins = Path('/tmp/lumine-primary-backend-bin')
def stop(p):
    if p is not None and p.poll() is None:
        p.terminate()
        try: p.wait(timeout=5)
        except subprocess.TimeoutExpired: p.kill(); p.wait()
def parse(text):
    maps = []
    current = None
    for line in text.splitlines():
        if re.match(r'^[0-9a-f]+-[0-9a-f]+ ', line):
            fields = line.split(maxsplit=5)
            current = {'name': fields[5] if len(fields)>5 else '[anonymous]', 'permissions': fields[1]}
            maps.append(current)
        elif current is not None:
            match = re.match(r'^(\w+):\s+(\d+) kB$',line)
            if match: current[match[1]] = int(match[2])
    return maps

display=next(n for n in range(140,180) if not Path(f'/tmp/.X11-unix/X{n}').exists() and not Path(f'/tmp/.X{n}-lock').exists())
env=dict(os.environ,DISPLAY=f':{display}',WINIT_UNIX_BACKEND='x11',LIBGL_ALWAYS_SOFTWARE='1',
    VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.json',WGPU_BACKEND='vulkan',
    DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent',LUMINE_MODE='idle',LUMINE_SECONDS='3',LUMINE_INITIAL_TICKS='0')
env.pop('WAYLAND_DISPLAY',None)
x=wm=app=None
results=[]
try:
    with (out/'xvfb.log').open('w') as log: x=subprocess.Popen(['Xvfb',f':{display}','-screen','0','1100x820x24'],stdout=log,stderr=subprocess.STDOUT)
    time.sleep(.5)
    with (out/'openbox.log').open('w') as log: wm=subprocess.Popen(['openbox'],env=env,stdout=log,stderr=subprocess.STDOUT)
    time.sleep(.5)
    for repeat,backend in [(r,b) for r in range(3) for b in (['vulkan','vulkan,gl'] if r%2==0 else ['vulkan,gl','vulkan'])]:
        name=f'{backend.replace(chr(44), chr(45))}-{repeat}'
        binary='component_workload'
        env['WGPU_BACKEND']=backend
        with (out/f'{name}.log').open('w') as log: app=subprocess.Popen([str(bins/binary)],env=env,stdout=log,stderr=subprocess.STDOUT)
        time.sleep(2)
        base=Path(f'/proc/{app.pid}')
        raw=(base/'smaps').read_text(); (out/f'{name}.smaps').write_text(raw)
        (out/f'{name}.rollup').write_text((base/'smaps_rollup').read_text())
        maps=parse(raw)
        totals={key:sum(m.get(key,0) for m in maps) for key in ['Rss','Pss','Private_Clean','Private_Dirty','Anonymous','Swap']}
        groups={}
        for m in maps:
            group=groups.setdefault(m['name'],dict(Rss=0,Pss=0,Anonymous=0))
            for key in group: group[key]+=m.get(key,0)
        results.append(dict(framework=name,backends=backend,repeat=repeat,binary_sha256=hashlib.sha256((bins/binary).read_bytes()).hexdigest(),
            snapshot_seconds_after_spawn=2,threads=len(list((base/'task').iterdir())),totals_kib=totals,
            mappings=sorted((dict(name=n,**v) for n,v in groups.items()),key=lambda m:m['Rss'],reverse=True)))
        assert app.wait(timeout=15)==0
        app=None
    (out/'summary.json').write_text(json.dumps(results,indent=2)+'\n')
finally:
    stop(app); stop(wm); stop(x)
