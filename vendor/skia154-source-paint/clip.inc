// Bounded rrect-only compatibility for the original clipping functions.
// Quarter conic subdivisions are monotonic in both axes; cubic/general path
// parsing is intentionally outside this ABI.
#define SkASSERTF(...) ((void)0)
#define SK_Scalar1 1.f
#define SkIsNaN std::isnan
#define SkIsFinite std::isfinite
#define SkDoubleToScalar float
struct SkRect {float fLeft,fTop,fRight,fBottom;};
struct float2 {
 float x,y;
 explicit float2(float s):x(s),y(s){}
 float2(float a,float b):x(a),y(b){}
};
float2 operator+(float2 a,float2 b){return {a.x+b.x,a.y+b.y};}
float2 operator-(float2 a,float2 b){return {a.x-b.x,a.y-b.y};}
float2 operator*(float2 a,float2 b){return {a.x*b.x,a.y*b.y};}
float2 from_point(SkPoint a){return {a.fX,a.fY};}
SkPoint to_point(float2 a){return {a.x,a.y};}
// SOURCE_CLIPPING_GEOMETRY
struct SkEdgeClipper {
 std::vector<std::array<SkPoint,3>> quads;
 std::vector<std::array<SkPoint,2>> lines;
 bool canCullToTheRight(){return true;}
 void appendVLine(float x,float y0,float y1,bool reverse){
  if(reverse)std::swap(y0,y1);
  lines.push_back({SkPoint{x,y0},SkPoint{x,y1}});
 }
 void appendQuad(const SkPoint p[3],bool reverse){
  quads.push_back({p[reverse?2:0],p[1],p[reverse?0:2]});
 }
 void clipMonoQuad(const SkPoint[],const SkRect&);
};
// SOURCE_CLIPPING_MONO_QUADS
