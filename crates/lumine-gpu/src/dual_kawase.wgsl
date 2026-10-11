// Pixel-space sampling clamps to the active crop, never pooled texture slack.
struct Params { size:vec4<f32>, effect:vec4<f32> }
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var original:texture_2d<f32>;
@group(0) @binding(2) var<uniform> params:Params;
@group(0) @binding(3) var linear:sampler;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{
 let p=vec2(f32((i<<1u)&2u),f32(i&2u))*2.-1.;return vec4(p,0.,1.);
}
fn tap(p:vec2<f32>)->vec4<f32>{
 let q=clamp(p,vec2(0.5),params.size.xy-0.5);
 return textureSampleLevel(source,linear,q/vec2<f32>(textureDimensions(source)),0.);
}
fn raw(p:vec2<f32>)->vec4<f32>{
 let q=clamp(p,vec2(0.5),params.size.zw-0.5);
 return textureSampleLevel(original,linear,q/vec2<f32>(textureDimensions(original)),0.);
}
@fragment fn fs_down(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{
 let q=p.xy*params.size.xy/params.size.zw;let d=vec2(params.effect.x*0.5);
 return (tap(q)*4.+tap(q+d)+tap(q-d)+tap(q+vec2(d.x,-d.y))+tap(q+vec2(-d.x,d.y)))/8.;
}
fn up(p:vec2<f32>)->vec4<f32>{
 let q=p*params.size.xy/params.size.zw;let d=params.effect.x*0.5;
 return (tap(q+vec2(-2.*d,0.))+tap(q+vec2(2.*d,0.))+tap(q+vec2(0.,-2.*d))+tap(q+vec2(0.,2.*d))
  +2.*(tap(q+vec2(-d,d))+tap(q+vec2(d,d))+tap(q+vec2(-d,-d))+tap(q+vec2(d,-d))))/12.;
}
@fragment fn fs_up(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{
 // The deepest reconstruction blends adjacent depths before the remaining upsamples.
 return mix(raw(p.xy),up(p.xy),params.effect.y);
}
