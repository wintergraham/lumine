struct PaintGeometry {
 matrix:array<vec4<f32>,2>, inverse:array<vec4<f32>,2>, fade_inverse:array<vec4<f32>,2>,
 bounds:vec4<f32>, metadata:vec4<u32>
}
struct PaintClip { inverse:array<vec4<f32>,2>, bounds:vec4<f32>, axes:vec4<u32>, corners:vec4<f32> }
struct PaintClips { values:array<PaintClip> }
@group(GEOMETRY_GROUP) @binding(0) var<uniform> geometry:PaintGeometry;
@group(GEOMETRY_GROUP) @binding(1) var<storage,read> paint_clips:PaintClips;
fn paint_point(rows:array<vec4<f32>,2>, p:vec2<f32>)->vec2<f32>{
 let q=vec3(p,1.);return vec2(dot(rows[0].xyz,q),dot(rows[1].xyz,q));
}
// Opt-in quad-pipeline A/B; blur composites retain the reference path.
override clip_interiors:bool=false;
fn paint_coverage(p:vec2<f32>)->f32{
 var coverage=1.;
 for(var i=0u;i<geometry.metadata.y;i++){
  let clip=paint_clips.values[geometry.metadata.x+i];let q=paint_point(clip.inverse,p);
  // Keep derivatives before all fragment-dependent interior branches.
  let pixel=fwidth(q);
  if !clip_interiors || any(clip.corners>vec4(0.)) {
   let delta=q-clip.bounds.xy-clip.bounds.zw*0.5;
   let radius=select(select(clip.corners.x,clip.corners.y,delta.x>0.),select(clip.corners.w,clip.corners.z,delta.x>0.),delta.y>0.);
   let edge=abs(delta)-clip.bounds.zw*0.5+vec2(radius);
   let width=max(max(pixel.x,pixel.y),0.5);
   // In this finite interior length(max(edge,0))=0 and distance<=-width.
   // A full AA width (rather than half) leaves the original clamp exactly 1.
   let interior=radius>=0. && all(edge<=vec2(-width)) && all(abs(vec4(edge,width,radius))<vec4(3.402823e38));
   if !clip_interiors || !interior {
    let distance=length(max(edge,vec2(0.)))+min(max(edge.x,edge.y),0.)-radius;
    if any(clip.corners>vec4(0.)) {coverage*=clamp(0.5-distance/width,0.,1.);}
   }
  }
  if clip.axes.x!=0u && (q.x<clip.bounds.x || q.x>=clip.bounds.x+clip.bounds.z){return 0.;}
  if clip.axes.y!=0u && (q.y<clip.bounds.y || q.y>=clip.bounds.y+clip.bounds.w){return 0.;}
 }
 return coverage;
}
