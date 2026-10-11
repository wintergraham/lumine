// `area`: the filter's visible pixels (x0, y0, x1, y1). Samples never leave
// the panel's visible crop: content outside a frosted panel does not
// bleed into its edge, and changes there never make it re-blur.
struct Params { direction:vec4<f32>, bounds:vec4<f32>, effect:vec4<f32>, area:vec4<f32> }
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var original:texture_2d<f32>;
@group(0) @binding(2) var<uniform> params:Params;
struct Samples { values:array<vec2<f32>> }
@group(0) @binding(3) var<storage,read> samples:Samples;
@group(0) @binding(4) var linear:sampler;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{let p=vec2(f32((i<<1u)&2u),f32(i&2u))*2.-1.;return vec4(p,0.,1.);}
fn tap(p:vec2<f32>)->vec4<f32>{
 // The allocation may exceed the active crop. Clamp texel centers before
 // linear sampling so stale pooled pixels cannot bleed into a panel's edge.
 var lo=params.area.xy+vec2(0.5);var hi=params.area.zw-vec2(0.5);
 if params.direction.w>1.5{lo=vec2(0.5);hi=params.area.zw-params.area.xy-vec2(0.5);}
 let q=clamp(p,lo,hi);
 return textureSampleLevel(source,linear,q/vec2<f32>(textureDimensions(source)),0.);
}
fn along(p:vec4<f32>)->vec4<f32>{
 var sum=tap(p.xy)*samples.values[0].y;
 for(var i=1u;i<arrayLength(&samples.values);i++){
  let sample=samples.values[i];let offset=params.direction.xy*sample.x;
  sum+=(tap(p.xy+offset)+tap(p.xy-offset))*sample.y;
 }
 return sum;
}
// Vertical pass: blur the horizontal result and mix it over the original.
fn vertical(p:vec4<f32>)->vec4<f32>{
 var pixel=p;
 // Mode 2 composites in world coordinates from crop-local scratch textures.
 if params.direction.w>1.5 && params.direction.w<2.5{pixel=vec4(p.xy-params.area.xy,p.zw);}
 if params.direction.w>2.5{return along(pixel);}
 let blurred=along(pixel);if params.direction.w<0.5{return blurred;}
 var amount=params.effect.x;if params.effect.y>0.{let d=min(p.y-params.bounds.y,params.bounds.y+params.bounds.w-p.y);amount*=clamp(d/params.effect.y,0.,1.);}
 return mix(textureLoad(original,vec2<i32>(pixel.xy),0),blurred,amount);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{return vertical(p);}
// Horizontal pass straight from the target: writes the horizontal blur and
// keeps the original pixels for the vertical mix, so no separate copy pass.
struct Split { @location(0) blurred:vec4<f32>, @location(1) original:vec4<f32> }
fn horizontal_pixel(p:vec4<f32>)->vec4<f32>{
 if params.direction.w< -0.5{return vec4(p.xy+params.area.xy,p.zw);}
 return p;
}
// Unequal scratch/cache extents cannot share MRT attachments. Keep their
// original crop through a shader draw, avoiding a Metal texture blit.
@fragment fn fs_horizontal(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{return along(horizontal_pixel(p));}
@fragment fn fs_original(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{return textureLoad(source,vec2<i32>(horizontal_pixel(p).xy),0);}
@fragment fn fs_split(@builtin(position) p:vec4<f32>)->Split{
 let pixel=horizontal_pixel(p);
 return Split(along(pixel),textureLoad(source,vec2<i32>(pixel.xy),0));
}
// Vertical pass drawn at presentation: the retained target and the drawable.
struct Both { @location(0) retained:vec4<f32>, @location(1) drawable:vec4<f32> }
@fragment fn fs_both(@builtin(position) p:vec4<f32>)->Both{
 let color=vertical(p);return Both(color,color);
}
