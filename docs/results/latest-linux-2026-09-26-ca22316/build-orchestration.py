#!/usr/bin/env python3
from pathlib import Path
import hashlib,json,os,subprocess,shutil,datetime
ROOT=Path(__file__).resolve().parents[3]; OUT=Path(__file__).resolve().parent
DEST=Path('/tmp/lumine-latest-linux-2026-09-26-ca22316-bin');DEST.mkdir(exist_ok=True)
if (OUT/'lumine-build-manifest.json').exists() or (OUT/'current.csv').exists():raise RuntimeError('refusing to overwrite frozen or measured evidence')
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def inventory():
 files=[ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'README.md']
 for base in ['crates','assets','comparisons','vendor']:
  files.extend(p for p in (ROOT/base).rglob('*') if p.is_file() and 'target' not in p.parts and '__pycache__' not in p.parts and (base=='vendor' or p.suffix in ['.rs','.toml','.lock','.wgsl','.ttf']))
 files.extend(ROOT/p for p in ['docs/composition.md','docs/styling.md','docs/window-controls.md'])
 return {str(p.relative_to(ROOT)):sha(p) for p in sorted(set(files))}
before=inventory();(OUT/'prebuild-source.json').write_text(json.dumps(before,indent=2)+'\n')
base=dict(os.environ,CARGO_PROFILE_DEV_DEBUG='0',CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='1')
records={};commands={}
for name,target,manifest,binary in [
 ('lumine','/tmp/lumine-target',None,'examples/component_workload'),
 ('gpui','/tmp/lumine-gpui-target','comparisons/gpui/Cargo.toml','lumine-compare-gpui'),
 ('quickgui','/tmp/lumine-quickgui-target','comparisons/quickgui/Cargo.toml','lumine-compare-quickgui')]:
 cmd=['cargo','build','--release','--locked']+(['--manifest-path',manifest] if manifest else ['-p','lumine-desktop','--example','component_workload'])
 env=dict(base,CARGO_TARGET_DIR=target)
 with (OUT/f'build-{name}.log').open('w') as log:subprocess.run(cmd,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
 assert inventory()==before,'source changed during build'
 p=Path(target)/'release'/binary;destination=DEST/p.name;shutil.copy2(p,destination)
 records[name]={'path':str(destination),'sha256':sha(destination),'bytes':destination.stat().st_size,'build_exit_code':0};commands[name]=cmd
 proof={'captured_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'source_sha256':before,'source_unchanged_across_verification_build':True,'build_command':cmd,'build_environment':{k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_PROFILE_DEV_DEBUG','CARGO_INCREMENTAL','CARGO_BUILD_JOBS']},'rustc':subprocess.check_output(['rustc','-Vv'],text=True),'binaries':{name:records[name]}}
 (OUT/f'{name}-build-manifest.json').write_text(json.dumps(proof,indent=2)+'\n');print('BUILT',name,flush=True)
refs={'source_sha256':{p:h for p,h in before.items() if p.startswith('comparisons/')},'binaries':{n:records[n] for n in ['gpui','quickgui']},'build_commands':commands,'freshly_rebuilt':True}
(OUT/'reference-build-manifest.json').write_text(json.dumps(refs,indent=2)+'\n')
