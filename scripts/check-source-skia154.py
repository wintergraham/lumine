#!/usr/bin/env python3
"""CPU-only native geometry/blit replay against frozen independent browser pixels.
This never grants native GPU/production/source qualification.
"""
from pathlib import Path
import argparse,hashlib,json,math,shutil,subprocess
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source',type=Path,required=True)
parser.add_argument('--manifest-sha256',required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--cxx',required=True)
parser.add_argument('--rustc',required=True)
args=parser.parse_args();root=Path(__file__).resolve().parents[1]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def tool(value):
 path=Path(shutil.which(value) or value).resolve();assert path.is_file(),value;return path
cxx=tool(args.cxx);rustc=tool(args.rustc)
source=args.source.resolve();manifest=source/'manifest.json'
assert sha(manifest)==args.manifest_sha256,'Original manifest identity changed'
contract=json.loads(manifest.read_text());assert contract['sourceOnly'] and not contract['nativeComparisonPerformed']
assert contract['kind']=='independent-source-rounded-paint-fixture'
assert contract['logicalViewport']==[96,80] and contract['dpr']==2
args.output.mkdir(parents=True,exist_ok=False);output=args.output.resolve()
cpp=root/'crates/lumine-gpu/src/source_skia154.cpp';obj=output/'source-skia154.o';archive=output/'libsource_skia154.a';binary=output/'source-skia154-replay'
commands=[[str(cxx),'-std=c++17','-O2','-ffp-contract=off','-c',str(cpp),'-o',str(obj)],
 ['ar','crs',str(archive),str(obj)],
 [str(rustc),'--edition','2024',str(root/'scripts/source-skia154-replay.rs'),'-o',str(binary),'-L','native='+str(output),'-l','static=source_skia154','-l','stdc++']]
for command in commands:subprocess.run(command,check=True,cwd=root,capture_output=True)
records=[];skipped=[]
for case in contract['cases']:
 nodes=case['nodes']
 if len(nodes)!=1 or case['background'][3]!=255 or nodes[0].get('opacity',1)!=1:
  skipped.append({'id':case['id'],'reason':'CPU replay slice covers one ordinary primitive over a known opaque backdrop only.'});continue
 node=nodes[0];x,y,w,h=node['bounds'];left=math.floor(x*2+.5);top=math.floor(y*2+.5);right=math.floor((x+w)*2+.5);bottom=math.floor((y+h)*2+.5)
 values=[left,top,right-left,bottom-top,node['radius']*2,*node['color'],*case['background'][:3]]
 result=subprocess.run([str(binary),*map(str,values)],check=True,capture_output=True)
 path=source/case['premultiplied']['path'];assert sha(path)==case['premultiplied']['sha256'],'Original source plane changed'
 expected=path.read_bytes();assert len(result.stdout)==len(expected)==192*160*4
 differences=sum(result.stdout[i:i+4]!=expected[i:i+4] for i in range(0,len(expected),4))
 records.append({'id':case['id'],'comparedPixels':192*160,'differingPixels':differences,'sourcePlaneSHA256':sha(path),'actualReplaySHA256':hashlib.sha256(result.stdout).hexdigest()})
assert records,'Empty replay cannot pass';assert sha(manifest)==args.manifest_sha256
inputs=['crates/lumine-gpu/src/source_skia154.cpp','crates/lumine-gpu/src/source_skia154.rs','crates/lumine-gpu/src/source_fill.rs','scripts/source-skia154-replay.rs','scripts/check-source-skia154.py']
report={'version':1,'kind':'independent-source-native-cpu-raster-replay','acceptance':False,'nativeGPUComparisonPerformed':False,'activationQualified':False,'sourceManifestSHA256':args.manifest_sha256,'inputSHA256':{path:sha(root/path)for path in inputs},'compilers':{'cxx':{'path':str(cxx),'sha256':sha(cxx)},'rustc':{'path':str(rustc),'sha256':sha(rustc)}},'commands':commands,'pixelTolerance':0,'fontSVGExceptions':[],'cases':records,'skipped':skipped,'differingPixels':sum(record['differingPixels']for record in records)}
(output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
for path in [obj,archive,binary]:path.unlink()
print(json.dumps({'report':str(output/'report.json'),'comparedCases':len(records),'skippedCases':len(skipped),'differingPixels':report['differingPixels'],'acceptance':False,'nativeGPUComparisonPerformed':False}))
if report['differingPixels']:raise SystemExit(1)
