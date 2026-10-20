// Cubics are unreachable: this ABI constructs only rrect lines and conics.
bool SkAnalyticCubicEdge::updateCubic(){return false;}
struct Edges {
 const SkRect* clip;
 explicit Edges(const SkRect* bounds):clip(bounds){}
 std::vector<SkAnalyticEdge*> list;
 std::vector<SkAnalyticQuadraticEdge*> quads;
 std::vector<SkAnalyticEdge*> lines;
 ~Edges(){for(auto*p:quads)delete p;for(auto*p:lines)delete p;}
 void raw_quad(SkPoint a,SkPoint b,SkPoint c){auto*q=new SkAnalyticQuadraticEdge{};quads.push_back(q);SkPoint p[]={a,b,c};if(q->setQuadratic(p))list.push_back(q);}
 void quad(SkPoint a,SkPoint b,SkPoint c){
  if(!clip){raw_quad(a,b,c);return;}
  SkPoint p[]={a,b,c};SkEdgeClipper clipped;clipped.clipMonoQuad(p,*clip);
  for(auto q:clipped.quads)raw_quad(q[0],q[1],q[2]);
  for(auto l:clipped.lines)line(l[0],l[1]);
 }
 // Only vertical rrect sides reach this method. Clamping their endpoint
 // coordinates is exactly the vertical specialization of SkLineClipper.
 void line(SkPoint a,SkPoint b){
  if(clip){a.fX=std::clamp(a.fX,clip->fLeft,clip->fRight);b.fX=std::clamp(b.fX,clip->fLeft,clip->fRight);a.fY=std::clamp(a.fY,clip->fTop,clip->fBottom);b.fY=std::clamp(b.fY,clip->fTop,clip->fBottom);}
  auto*q=new SkAnalyticEdge{};lines.push_back(q);if(q->setLine(a,b))list.push_back(q);
 }
 // SkGeometry.cpp SkConic::chop and subdivide, with scalar float2 lanes.
 void conic(SkPoint a,SkPoint b,SkPoint c,float w,int level){
  if(level==0){quad(a,b,c);return;}
  float scale=1/(1+w),newW=sqrtf(0.5f+0.5f*w);
  // Chromium's pinned SkUserConfig explicitly enables this legacy branch.
  SkPoint wp1={w*b.fX,w*b.fY};
  SkPoint p1={(a.fX+wp1.fX)*scale,(a.fY+wp1.fY)*scale};
  SkPoint p3={(wp1.fX+c.fX)*scale,(wp1.fY+c.fY)*scale};
  SkPoint p2={(a.fX+2*wp1.fX+c.fX)*scale*0.5f,(a.fY+2*wp1.fY+c.fY)*scale*0.5f};
  if((a.fY-b.fY)*(c.fY-b.fY)<=0){
   if((a.fY-p2.fY)*(c.fY-p2.fY)>0)p2.fY=fabsf(p2.fY-a.fY)<fabsf(p2.fY-c.fY)?a.fY:c.fY;
   if((a.fY-p1.fY)*(p2.fY-p1.fY)>0)p1.fY=a.fY;
   if((p2.fY-p3.fY)*(c.fY-p3.fY)>0)p3.fY=c.fY;
  }
  conic(a,p1,p2,newW,level-1);conic(p2,p3,c,newW,level-1);
 }
 void corner(SkPoint a,SkPoint b,SkPoint c){
  float w=sqrtf(0.5f),k=(w-1)/(4*(1+w));
  float x=k*(a.fX-2*b.fX+c.fX),y=k*(a.fY-2*b.fY+c.fY);
  float error=sqrtf(x*x+y*y);int level=0;
  while(error>0.25f&&level<5){error*=0.25f;level++;}
  conic(a,b,c,w,level);
 }
};
} // namespace ask_skia154
static size_t rrect_impl(
 float left,float top,float width,float height,float radius,
 int viewport_width,int viewport_height,const ask_skia154::SkRect* clip,
 ask_skia154::Span* output,size_t capacity) noexcept {
 using namespace ask_skia154;
 // The original scan algorithm uses 16.16 coordinates and short run lengths.
 // Out-of-slice geometry explicitly falls back; it never reaches unsafe casts.
 if(!std::isfinite(left)||!std::isfinite(top)||!std::isfinite(width)||!std::isfinite(height)||!std::isfinite(radius)||
    width<1||height<1||left<0||top<0||left+width>8191||top+height>8191||radius<0||
    viewport_width<1||viewport_width>8191||viewport_height<1||viewport_height>8191)return 0;
 // Small mask-backed draws use a different blit stage; do not approximate it.
 if(width<=32&&std::ceil(width/4)*4*height<=1024)return 0;
 try {
  float right=left+width,bottom=top+height;
  radius=std::min(radius,std::min(width,height)*0.5f);
  const bool contained=!clip||(clip->fLeft<=left&&clip->fTop<=top&&clip->fRight>=right&&clip->fBottom>=bottom);
  Edges edges(contained?nullptr:clip);
  if(radius>0){
   edges.corner({left,top+radius},{left,top},{left+radius,top});
   edges.corner({right-radius,top},{right,top},{right,top+radius});
   edges.corner({right,bottom-radius},{right,bottom},{right-radius,bottom});
   edges.corner({left+radius,bottom},{left,bottom},{left,bottom-radius});
  }
  edges.line({left,top+radius},{left,bottom-radius});
  edges.line({right,top+radius},{right,bottom-radius});
  if(edges.list.size()<2)return 0;
  std::sort(edges.list.begin(),edges.list.end(),compare_edges);
  SkAnalyticEdge head{},tail{};tail.fUpperY=INT32_MAX;tail.fLowerY=INT32_MAX;head.fNext=edges.list[0];
  for(size_t i=0;i<edges.list.size();i++){edges.list[i]->fPrev=i?edges.list[i-1]:&head;edges.list[i]->fNext=i+1<edges.list.size()?edges.list[i+1]:&tail;}
  tail.fPrev=edges.list.back();
  Context context;context.width=viewport_width;context.height=viewport_height;
  AdditiveBlitter real(context,true),additive(context,false,&real);
  int start=std::max(int(floorf(top)),clip?int(clip->fTop):0);
  int stop=std::min(int(ceilf(bottom)),clip?int(clip->fBottom):viewport_height);
  aaa_walk_convex_edges(&head,&additive,start,stop,
   SkIntToFixed(clip?int(clip->fLeft):0),SkIntToFixed(clip?int(clip->fRight):viewport_width),false);
  additive.flush();
  if(context.overflow)return 0;
  if(output&&capacity>=context.spans.size())std::copy(context.spans.begin(),context.spans.end(),output);
  return context.spans.size();
 }catch(...){return 0;}
}
extern "C" size_t lumine_source_skia154_rrect(
 float left,float top,float width,float height,float radius,
 int viewport_width,int viewport_height,ask_skia154::Span* output,size_t capacity) noexcept {
 return rrect_impl(left,top,width,height,radius,viewport_width,viewport_height,nullptr,output,capacity);
}
extern "C" size_t lumine_source_skia154_rrect_clipped(
 float left,float top,float width,float height,float radius,
 int viewport_width,int viewport_height,int clip_left,int clip_top,int clip_right,int clip_bottom,
 ask_skia154::Span* output,size_t capacity) noexcept {
 if(clip_left<0||clip_top<0||clip_right>viewport_width||clip_bottom>viewport_height||
    clip_right<=clip_left||clip_bottom<=clip_top)return 0;
 ask_skia154::SkRect clip{float(clip_left),float(clip_top),float(clip_right),float(clip_bottom)};
 return rrect_impl(left,top,width,height,radius,viewport_width,viewport_height,&clip,output,capacity);
}
