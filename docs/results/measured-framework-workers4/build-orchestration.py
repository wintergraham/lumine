from pathlib import Path
import hashlib,json,os,subprocess,shutil,datetime
r=Path('/home/ubuntu/GitHub/lumine'); out=r/'docs/results/measured-framework-workers4'; dest=Path('/tmp/lumine-measured-framework-workers4-bin'); dest.mkdir(exist_ok=True)
if (out/'lumine-build-manifest.json').exists() or (out/'current.csv').exists():
 raise RuntimeError('refusing to replace existing frozen or measured artifacts')
files=[r/'Cargo.toml',r/'Cargo.lock',r/'README.md']
for base in ('crates','assets','comparisons/workload'):
 files.extend(p for p in (r/base).rglob('*') if p.is_file() and 'target' not in p.parts and p.suffix in ('.rs','.toml','.lock','.wgsl','.ttf'))
files.extend(r/p for p in ('docs/composition.md','docs/styling.md','docs/window-controls.md'))
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
before={str(p.relative_to(r)):sha(p) for p in sorted(set(files))}
env=dict(os.environ,CARGO_TARGET_DIR='/tmp/lumine-target',CARGO_PROFILE_DEV_DEBUG='0',CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='1')
cmd=['cargo','build','--release','-p','lumine-desktop','--example','component_workload','--locked']
with (out/'build.log').open('w') as log: subprocess.run(cmd,cwd=r,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
assert before=={str(p.relative_to(r)):sha(p) for p in sorted(set(files))}
shutil.copy2('/tmp/lumine-target/release/examples/component_workload',dest/'component_workload')
refs=json.loads((r/'docs/results/component-comparison/adapters-build-manifest.json').read_text())
for relative,expected in refs['source_sha256'].items():
 assert sha(r/relative)==expected, 'reference source changed: '+relative
for name,record in refs['binaries'].items():
 p=Path(record['path']); assert sha(p)==record['sha256']; shutil.copy2(p,dest/p.name)
p=dest/'component_workload'
proof=dict(captured_at_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),source_sha256=before,source_unchanged_across_verification_build=True,build_command=cmd,build_environment={k:env[k] for k in ('CARGO_TARGET_DIR','CARGO_PROFILE_DEV_DEBUG','CARGO_INCREMENTAL','CARGO_BUILD_JOBS')},rustc=subprocess.check_output(['rustc','-Vv'],text=True),binaries={'lumine':dict(path=str(p),sha256=sha(p),bytes=p.stat().st_size,build_exit_code=0)})
(out/'lumine-build-manifest.json').write_text(json.dumps(proof,indent=2)+'\n')
print('Frozen',len(before),'inputs',proof['binaries'])
