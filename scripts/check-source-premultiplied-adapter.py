#!/usr/bin/env python3
"""CPU-only replay of the actual source RGBA adapter against frozen source.
The explicit prebuilt Lumine dependency supplies Scene/layout only. No GPU,
Cargo, production capability, expected-pixel input to the renderer, or inferred
native target identity is used. A later actual GPU comparison remains required.
"""
from pathlib import Path
import argparse,hashlib,json,shutil,subprocess
p=argparse.ArgumentParser(description=__doc__)
for name in ['source','output','dependency-dir','lumine-rlib','rustc-hash-rlib']:
 p.add_argument('--'+name,type=Path,required=True)
for name in ['manifest-sha256','cxx','rustc']:p.add_argument('--'+name,required=True)
a=p.parse_args();root=Path(__file__).resolve().parents[1]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def tool(value):
 path=Path(shutil.which(value)or value).resolve();assert path.is_file();return path
cxx=tool(a.cxx);rustc=tool(a.rustc);source=a.source.resolve();manifest=source/'manifest.json'
assert sha(manifest)==a.manifest_sha256
contract=json.loads(manifest.read_text())
assert contract['kind']=='independent-source-rounded-paint-fixture' and contract['sourceOnly'] and not contract['nativeComparisonPerformed']
assert contract['logicalViewport']==[96,80] and contract['dpr']==2 and len(contract['cases'])==20
a.output.mkdir(parents=True,exist_ok=False);out=a.output.resolve()
cpp=root/'crates/lumine-gpu/src/source_skia154.cpp';obj=out/'adapter.o';archive=out/'libadapter.a';binary=out/'adapter'
commands=[[str(cxx),'-std=c++17','-O2','-ffp-contract=off','-c',str(cpp),'-o',str(obj)],['ar','crs',str(archive),str(obj)],
 [str(rustc),'--edition','2024',str(root/'scripts/source-premultiplied-adapter-replay.rs'),'-L','dependency='+str(a.dependency_dir.resolve()),'--extern','lumine='+str(a.lumine_rlib.resolve()),'--extern','rustc_hash='+str(a.rustc_hash_rlib.resolve()),'-L','native='+str(out),'-l','static=adapter','-l','stdc++','-o',str(binary)]]
for command in commands:subprocess.run(command,check=True,capture_output=True,cwd=root)
records=[];unsupported=[]
for case in contract['cases']:
 assert case['fullViewportRepeatEqual'] and case['geometryRepeatEqual']
 values=[*case['background'],len(case['nodes'])];ids=[node['id']for node in case['nodes']]
 for node in case['nodes']:
  values.extend([ids.index(node['parent'])if'parent'in node else -1,*node['bounds'],node['radius'],*node['color'],int(node.get('clip',False)),node.get('opacity',1)])
 run=subprocess.run([str(binary),*map(str,values)],capture_output=True)
 if run.returncode==3:
  unsupported.append({'id':case['id'],'reason':'Actual source RGBA adapter rejected this paint transaction.'});continue
 assert run.returncode==0,run.stderr.decode()
 expected_path=source/case['premultiplied']['path'];assert sha(expected_path)==case['premultiplied']['sha256']
 expected=expected_path.read_bytes();actual=run.stdout;assert len(actual)==len(expected)==192*160*4
 differences=sum(actual[i:i+4]!=expected[i:i+4]for i in range(0,len(expected),4))
 records.append({'id':case['id'],'comparedPixels':192*160,'differingPixels':differences,'sourceSHA256':sha(expected_path),'actualSHA256':hashlib.sha256(actual).hexdigest()})
assert records and sha(manifest)==a.manifest_sha256
inputs=['crates/lumine/src/source_paint.rs','crates/lumine-gpu/src/source_rgba_paint.rs','crates/lumine-gpu/src/source_fill_paint.rs','crates/lumine-gpu/src/source_fill.rs','crates/lumine-gpu/src/source_skia154.rs','crates/lumine-gpu/src/source_skia154.cpp','scripts/source-premultiplied-adapter-replay.rs','scripts/check-source-premultiplied-adapter.py']
report={'version':1,'kind':'independent-source-premultiplied-adapter-cpu-replay','acceptance':False,'activationQualified':False,'nativeGPUComparisonPerformed':False,'sourceManifestSHA256':a.manifest_sha256,'pixelTolerance':0,'fontSVGExceptions':[],'transparentZeroAlphaPrimitivesOmitted':True,'negativeChecks':['unknown native translucency','unknown layer','unknown image','unknown transformed draw'],'cases':records,'unsupported':unsupported,'differingPixels':sum(c['differingPixels']for c in records),'inputs':{path:sha(root/path)for path in inputs},'compilers':{str(path):sha(path)for path in [cxx,rustc]},'explicitSceneDependencies':{str(path.resolve()):sha(path.resolve())for path in [a.lumine_rlib,a.rustc_hash_rlib]},'commands':commands}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
for path in [obj,archive,binary]:path.unlink()
print(json.dumps({'report':str(out/'report.json'),'comparedCases':len(records),'unsupportedCases':len(unsupported),'differingPixels':report['differingPixels'],'acceptance':False}))
if report['differingPixels']:raise SystemExit(1)
