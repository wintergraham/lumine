import os, pathlib, subprocess, time, json, hashlib, sys, tarfile, platform
sys.path.insert(0, str(pathlib.Path('scripts').resolve()))
from compare import sample
out=pathlib.Path('docs/results/component-release-smoke'); binary=pathlib.Path('/tmp/lumine-target/release/examples/component_workload')
files=sorted(p for base in ('crates','comparisons/workload') for p in pathlib.Path(base).rglob('*') if p.is_file() and p.suffix in ('.rs','.toml','.lock','.wgsl') and 'target' not in p.parts)+[pathlib.Path('Cargo.toml'),pathlib.Path('Cargo.lock')]
metadata={'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'platform':platform.platform(),'source_sha256':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in files},'seconds':5,'warmup_seconds':1,'note':'Single-run release smoke, Xvfb llvmpipe software Vulkan. Not a hardware GPU benchmark or a controlled multi-framework comparison.'}
with tarfile.open(out/'source.tar.gz','w:gz') as archive:
 for p in files: archive.add(p,arcname=str(p))
read_fd,write_fd=os.pipe()
xlog=open(out/'xvfb.log','w'); xvfb=subprocess.Popen(['Xvfb','-displayfd',str(write_fd),'-screen','0','1280x900x24','-nolisten','tcp'],pass_fds=(write_fd,),stdout=xlog,stderr=xlog);os.close(write_fd)
wm=None
try:
 display=':'+os.read(read_fd,64).decode().strip();os.close(read_fd)
 env=dict(os.environ,DISPLAY=display,WINIT_UNIX_BACKEND='x11',WGPU_BACKEND='vulkan',VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.json',LIBGL_ALWAYS_SOFTWARE='1',LUMINE_INITIAL_TICKS='0');env.pop('WAYLAND_DISPLAY',None)
 metadata['environment']={k:env.get(k) for k in ('DISPLAY','WINIT_UNIX_BACKEND','WGPU_BACKEND','VK_ICD_FILENAMES','LIBGL_ALWAYS_SOFTWARE')}
 (out/'metadata.json').write_text(json.dumps(metadata,indent=2))
 wlog=open(out/'openbox.log','w');wm=subprocess.Popen(['openbox'],env=env,stdout=wlog,stderr=wlog);time.sleep(.5)
 summaries=[]
 for mode in ('idle','stream','scroll','both'):
  samples=[];start=time.monotonic()
  with open(out/(mode+'.log'),'w') as log:
   proc=subprocess.Popen([str(binary)],env=dict(env,LUMINE_MODE=mode,LUMINE_SECONDS='5'),stdout=log,stderr=log)
   try:
    while proc.poll() is None:
     elapsed=time.monotonic()-start
     if elapsed>25:raise RuntimeError('application timeout')
     value=sample(proc.pid)
     if value and elapsed>=1:samples.append({'elapsed_seconds':elapsed,'cpu_seconds':value[0],'rss_bytes':value[1]})
     time.sleep(.05)
   finally:
    if proc.poll() is None:proc.kill()
    code=proc.wait()
  reports=[]
  for line in (out/(mode+'.log')).read_text().splitlines():
   try: reports.append(json.loads(line))
   except json.JSONDecodeError:pass
  if code or len(samples)<2 or not reports: raise RuntimeError(f'{mode} failure {code}')
  wall=samples[-1]['elapsed_seconds']-samples[0]['elapsed_seconds'];cpu=samples[-1]['cpu_seconds']-samples[0]['cpu_seconds']
  summary={'mode':mode,'exit_code':code,'cpu_percent_one_core':100*cpu/wall,'mean_rss_mib':sum(s['rss_bytes'] for s in samples)/len(samples)/1048576,'peak_rss_mib':max(s['rss_bytes'] for s in samples)/1048576,'sample_count':len(samples),'application_reports':reports}
  (out/(mode+'.json')).write_text(json.dumps({'summary':summary,'samples':samples},indent=2));summaries.append(summary);print(json.dumps(summary),flush=True)
 (out/'summary.json').write_text(json.dumps(summaries,indent=2))
finally:
 for proc in (wm,xvfb):
  if proc is not None:
   proc.terminate()
   try:proc.wait(timeout=3)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
