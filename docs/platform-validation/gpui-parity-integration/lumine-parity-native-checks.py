import pathlib,subprocess,os,json,time,hashlib
root=pathlib.Path('/home/ubuntu/GitHub/lumine');out=pathlib.Path('/tmp/lumine-parity-native');out.mkdir(exist_ok=False)
binaries=pathlib.Path('/tmp/lumine-target/debug/examples')
env=dict(os.environ,LD_LIBRARY_PATH='/tmp/lumine-ci-loader-check/build/loader',LP_NUM_THREADS='4')
checks=[('paint-media',['xvfb-run','-a','-s','-screen 0 1000x700x24','python3','scripts/paint_media_smoke.py','--binaries',str(binaries)]),('cursors',['xvfb-run','-a','-s','-screen 0 1000x700x24','python3','scripts/cursors_smoke.py',str(binaries/'cursors')])]
for script,binary,extra,name in [('rich_text','rich_text',[],'rich-text'),('layout','layout',[],'layout'),('actions','actions',[],'actions'),('drag_drop','drag_drop',[],'drag-drop'),('file_drop','drag_drop',[],'file-drop-x11'),('wayland_file_drop','drag_drop',[],'file-drop-wayland'),('native_menu','dynamic_menus',[],'menus-x11'),('native_menu','dynamic_menus',['--wayland'],'menus-wayland'),('effects','effects',[],'effects'),('component_host','component_workload',[],'component-host')]:
 checks.append((name,['python3','scripts/'+script+'_smoke.py',str(binaries/binary)]+extra))
results=[]
for name,command in checks:
 command+=['--output',str(out/name)];start=time.monotonic();print('START',name,flush=True)
 with (out/(name+'.log')).open('w') as log:r=subprocess.run(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=120)
 results.append(dict(name=name,command=command,exit_code=r.returncode,seconds=time.monotonic()-start));(out/'results.json').write_text(json.dumps(results,indent=2)+'\n');print('DONE',name,r.returncode,flush=True)
 if r.returncode:raise SystemExit(r.returncode)
(out/'binary-hashes.json').write_text(json.dumps({p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in binaries.iterdir() if p.is_file() and p.suffix=='' and '-' not in p.name},indent=2)+'\n')
