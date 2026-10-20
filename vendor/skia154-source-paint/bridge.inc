// Cubics are unreachable: this ABI constructs only rrect lines and conics.
bool SkAnalyticCubicEdge::updateCubic(){return false;}
struct Edges {
 std::vector<SkAnalyticEdge*> list;
 std::vector<SkAnalyticQuadraticEdge*> quads;
 std::vector<SkAnalyticEdge*> lines;
 ~Edges(){for(auto*p:quads)delete p;for(auto*p:lines)delete p;}
 void quad(SkPoint a,SkPoint b,SkPoint c){auto*q=new SkAnalyticQuadraticEdge{};quads.push_back(q);SkPoint p[]={a,b,c};if(q->setQuadratic(p))list.push_back(q);}
 void line(SkPoint a,SkPoint b){auto*q=new SkAnalyticEdge{};lines.push_back(q);if(q->setLine(a,b))list.push_back(q);}
 // SkGeometry.cpp SkConic::chop and subdivide, with scalar float2 lanes.
 void conic(SkPoint a,SkPoint b,SkPoint c,float w,int level){
  if(level==0){quad(a,b,c);return;}
  float scale=1/(1+w),newW=sqrtf(0.5f+0.5f*w);
  SkPoint t0={a.fX*scale,a.fY*scale},t1={b.fX*(w*scale),b.fY*(w*scale)},t2={c.fX*scale,c.fY*scale};
  SkPoint p1={t0.fX+t1.fX,t0.fY+t1.fY},p3={t1.fX+t2.fX,t1.fY+t2.fY},p2={0.5f*t0.fX+t1.fX+0.5f*t2.fX,0.5f*t0.fY+t1.fY+0.5f*t2.fY};
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
extern "C" size_t lumine_source_skia154_rrect(
 float left,float top,float width,float height,float radius,
 int viewport_width,int viewport_height,ask_skia154::Span* output,size_t capacity) noexcept {
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
  Edges edges;
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
  aaa_walk_convex_edges(&head,&additive,floorf(top),ceilf(bottom),0,SkIntToFixed(viewport_width),false);
  additive.flush();
  if(context.overflow)return 0;
  if(output&&capacity>=context.spans.size())std::copy(context.spans.begin(),context.spans.end(),output);
  return context.spans.size();
 }catch(...){return 0;}
}
