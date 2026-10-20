#!/usr/bin/env python3
"""Run the real private source-copy classifier/batcher regression without a GPU.
Exact production functions/tests are extracted unchanged. Only unused GPU bind
handles and Draw fields are represented by CPU stubs; this is not an ABI,
renderer pixel replay, or GPU/production qualification test.
"""
from pathlib import Path
import argparse,hashlib,json,shutil,subprocess
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--output',type=Path,required=True);p.add_argument('--rustc',required=True)
a=p.parse_args();root=Path(__file__).resolve().parents[1]
paths=['crates/lumine-gpu/src/lib.rs','crates/lumine-gpu/src/draw_diagnostics.rs','crates/lumine-gpu/src/source_rgba_paint.rs','crates/lumine/src/scene.rs','scripts/check-source-copy-batching.py']
texts={path:(root/path).read_text()for path in paths}
def block(text,needle):
 start=text.index(needle);opening=text.index('{',start);depth=0
 for i in range(opening,len(text)):
  if text[i]=='{':depth+=1
  elif text[i]=='}':
   depth-=1
   if depth==0:return text[start:i+1]
 raise AssertionError('Unclosed source block: '+needle)
lib=texts[paths[0]];diag=texts[paths[1]];rgba=texts[paths[2]];scene=texts[paths[3]]
enum=block(lib,'enum Shading {');quad=block(lib,'struct Quad {')
rect=block(scene,'pub struct Rect {')+block(scene,'impl Rect {')
classify=block(lib,'fn gaussian_shadow(')+block(lib,'fn shading(')
batch=block(diag,'pub(super) fn batches(')
marker=next(line for line in rgba.splitlines()if line.startswith('const RESOLVED:'))
resolved=block(rgba,'pub(super) fn resolved(')
unit=block(rgba,'fn resolved_transparent_rgba_keeps_copy_semantics_through_batching(')
draw=block(diag,'fn draw(start:')
tests=[block(diag,'fn '+name+'(')for name in ['batches_preserve_order_merging_clip_flush_and_geometry_binding','premultiplied_copy_cannot_be_downgraded_for_small_damage']]
stubs='''#![allow(dead_code)]
#[derive(Clone,Copy)]struct Transform{x:f32,y:f32}
mod lumine{pub mod affine{#[derive(Clone,Copy)]pub struct Affine;impl Affine{pub const IDENTITY:Self=Self;}}}
use lumine::affine::Affine;
struct Draw{diagnostic_source:Option<()>,transform:Affine,geometry_offset:u32,geometry_hash:u64,geometry:Option<()>,shading:Shading,blur:Option<()>,blur_mask:Option<()>,start:u32,end:u32,clip:Rect,bounds:Rect,image:Option<u64>,layer:Option<u64>}
struct Batch{geometry_offset:u32,geometry:Option<()>,rect:Rect,source:(Option<u64>,Option<u64>),shading:Shading,start:u32,end:u32}
const NO_MASK:[f32;4]=[0.;4];
'''
code=stubs+'#[derive(Clone,Copy,Debug,PartialEq)]\n'+rect+'\n#[derive(Clone,Copy,Debug,PartialEq,Eq)]\n'+enum+'\n'+quad+'\n'+classify
code+='\nmod draw_diagnostics{use super::*;'+batch+'}\nuse draw_diagnostics::batches;'
code+='\nmod source_rgba_paint{use super::*;'+marker+resolved+'\nmod tests{use super::*;#[test]\n'+unit+'}}'
code+='\nmod tests{use super::*;'+draw+'\n'+''.join('#[test]\n'+test+'\n'for test in tests)+'}'
a.output.mkdir(parents=True,exist_ok=False);out=a.output.resolve();source=out/'source-copy-batching.rs';binary=out/'source-copy-batching';source.write_text(code)
rustc=Path(shutil.which(a.rustc)or a.rustc).resolve();commands=[[str(rustc),'--edition','2024','--test',str(source),'-o',str(binary)],[str(binary),'--test-threads=1']]
logs=[]
for command in commands:
 run=subprocess.run(command,capture_output=True,text=True);logs.append({'command':command,'exitCode':run.returncode,'stdout':run.stdout,'stderr':run.stderr})
 if run.returncode:raise RuntimeError(run.stderr or run.stdout)
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
report={'version':1,'kind':'production-source-copy-batching-cpu-regression','acceptance':False,'activationQualified':False,'nativeGPUComparisonPerformed':False,'renderedPixels':0,'testCount':3,'inputs':{path:sha(root/path)for path in paths},'rustc':{'path':str(rustc),'sha256':sha(rustc)},'extractedTestSourceSHA256':sha(source),'runs':logs}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');binary.unlink()
print(json.dumps({'report':str(out/'report.json'),'testCount':3,'passed':True,'nativeGPUComparisonPerformed':False,'acceptance':False}))
