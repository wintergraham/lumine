struct Params { bounds:vec4<f32>, effect:vec4<f32>, mask:vec4<f32> }
@group(0) @binding(0) var filtered:texture_2d<f32>;
@group(0) @binding(1) var original:texture_2d<f32>;
@group(0) @binding(2) var<uniform> params:Params;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{
 let p=vec2(f32((i<<1u)&2u),f32(i&2u))*2.-1.;return vec4(p,0.,1.);
}
fn composite(p:vec4<f32>)->vec4<f32>{
 let local=vec2<i32>(p.xy-params.effect.zw);var amount=params.effect.x;
  var edge_y=p.y;var edge_bounds=params.bounds;var fade_y=p.y;
  if geometry.metadata.w!=0u {
   let scale=bitcast<f32>(geometry.metadata.z);let world=p.xy/scale;
   let q=paint_point(geometry.inverse,world);let b=geometry.bounds;
   amount*=paint_coverage(world);
   if q.x<b.x || q.y<b.y || q.x>=b.x+b.z || q.y>=b.y+b.w {amount=0.;}
   edge_y=q.y*scale;edge_bounds=b*scale;fade_y=paint_point(geometry.fade_inverse,world).y*scale;
  }
 if params.effect.y>0.{let d=min(edge_y-edge_bounds.y,edge_bounds.y+edge_bounds.w-edge_y);amount*=clamp(d/params.effect.y,0.,1.);}
 if params.mask.z>0.{let t=clamp((fade_y-params.mask.x)/params.mask.z,0.,1.);amount*=t*t;}
 if params.mask.w>0.{let b=clamp((params.mask.y-fade_y)/params.mask.w,0.,1.);amount*=b*b;}
 return mix(textureLoad(original,local,0),textureLoad(filtered,local,0),amount);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{return composite(p);}
struct Both { @location(0) retained:vec4<f32>, @location(1) drawable:vec4<f32> }
@fragment fn fs_both(@builtin(position) p:vec4<f32>)->Both{let c=composite(p);return Both(c,c);}
