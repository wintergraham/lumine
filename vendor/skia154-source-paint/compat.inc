
#include <cstdint>
#include <cmath>
#include <algorithm>
#include <vector>
#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <climits>
#include <iterator>
#include <array>
#include <cstring>
#include <optional>
using SkFixed=int32_t;using SkFDot6=int32_t;using SkScalar=float;using SkAlpha=uint8_t;
#define SkASSERT(condition) ((void)0)
#define SkDEBUGCODE(x)
#define SK_Fixed1 65536
#define SK_FixedHalf 32768
#define SK_MaxS32 INT32_MAX
#define SK_MinS32 INT32_MIN
#define SkToS8 int8_t
#define SkToU8 uint8_t
#define SkToS32 int32_t
template<class T,class U> T SkTo(U x){return T(x);}
template<class T> auto SkAbs32(T x){return std::abs(x);}
template<class T> T SkLeftShift(T x,int n){return T(uint64_t(x)<<n);}
int SkCLZ(uint32_t x){return x?__builtin_clz(x):32;}
SkFixed SkFixedMul(SkFixed a,SkFixed b){return (int64_t(a)*b)>>16;}
SkFixed SkFixedDiv(SkFixed a,SkFixed b){return std::clamp(int64_t(a)*65536/b,int64_t(INT32_MIN),int64_t(INT32_MAX));}
#define SkFixedFloorToInt(x) ((x)>>16)
#define SkFixedCeilToInt(x) (((x)+65535)>>16)
#define SkFixedCeilToFixed(x) (((x)+65535)&~65535)
#define SkFixedFloorToFixed(x) ((x)&~65535)
#define SkFixedRoundToInt(x) (((x)+32768)>>16)
#define SkFixedRoundToFixed(x) (((x)+32768)&~65535)
#define SkIntToFixed(x) ((x)*65536)
#define SkFDot6ToFixed(x) ((x)*1024)
#define SkFixedToFDot6(x) ((x)>>10)
#define SkFDot6Round(x) (((x)+32)>>6)
#define SkScalarToFDot6(x) int32_t((x)*64)
#define SkFDot6Div SkFixedDiv
SkFixed Sk32_sat_add(SkFixed a,SkFixed b){return std::clamp(int64_t(a)+b,int64_t(INT32_MIN),int64_t(INT32_MAX));}
SkFixed Sk32_sat_sub(SkFixed a,SkFixed b){return std::clamp(int64_t(a)-b,int64_t(INT32_MIN),int64_t(INT32_MAX));}
template<class T,class F> void SkTQSort(T*a,T*b,F f){std::sort(a,b+1,f);}
struct SkPoint {float fX,fY;};

namespace ask_skia154 {
// SOURCE_RUN_BASED_ALPHA_SNAP
void safely_add_alpha(SkAlpha*a,SkAlpha b){*a=std::min(255,int(*a)+b);}
struct Span {int32_t x,y,width,coverage,blend;};
struct Context {std::vector<Span> spans;int width,height;bool overflow=false;
 void emit(int x,int y,int n,int alpha,int blend){
  if(alpha<=0||n<=0||y<0||y>=height)return;int right=std::min(width,x+n);x=std::max(0,x);if(right<=x)return;
  if(spans.size()>=65536){overflow=true;return;}spans.push_back({x,y,right-x,alpha,blend});
 }
};
struct AdditiveBlitter {
 Context& context;int row=-1;std::vector<int> alphas;bool real;AdditiveBlitter* other;
 AdditiveBlitter(Context&c,bool r,AdditiveBlitter*o=nullptr):context(c),alphas(c.width),real(r),other(o){}
 AdditiveBlitter* getRealBlitter(bool=false){return real?this:other;}
 // Original RunBasedAdditiveBlitter::flush applies snapAlpha to accumulated
 // coverage before calling the real blitter. Direct H2/V/rect paths bypass it.
 void flush(){if(row>=0){for(int x=0;x<context.width;x++){SkAlpha alpha=snapAlpha(alphas[x]);if(alpha)context.emit(x,row,1,alpha,0);}std::fill(alphas.begin(),alphas.end(),0);}}
 void blitAntiH(int x,int y,SkAlpha alpha){if(real)context.emit(x,y,1,alpha,0);else{if(row!=y){flush();row=y;}if(x>=0&&x<context.width)alphas[x]=std::min(255,alphas[x]+alpha);}}
 void blitAntiH(int x,int y,int n,SkAlpha alpha){if(real)context.emit(x,y,n,alpha,0);else for(int i=0;i<n;i++)blitAntiH(x+i,y,alpha);}
 void blitAntiH(int x,int y,const SkAlpha*alpha,int n){for(int i=0;i<n;i++)blitAntiH(x+i,y,alpha[i]);}
 void blitAntiH(int x,int y,const SkAlpha*alpha,const int16_t*runs){for(int i=0;runs[i];i+=runs[i])blitAntiH(x+i,y,runs[i],alpha[i]);}
 void blitH(int x,int y,int n){blitAntiH(x,y,n,255);}
 void blitV(int x,int y,int height,SkAlpha alpha){for(int j=0;j<height;j++)blitAntiH(x,y+j,alpha);}
 void blitAntiH2(int x,int y,SkAlpha left,SkAlpha right){context.emit(x,y,1,left,1);context.emit(x+1,y,1,right,1);}
 void blitAntiRect(int x,int y,int width,int height,SkAlpha left,SkAlpha right){for(int j=0;j<height;j++){blitAntiH(x,y+j,left);blitH(x+1,y+j,width);blitAntiH(x+width+1,y+j,right);}}
 void flush_if_y_changed(SkFixed y,SkFixed nextY){if((y>>16)!=(nextY>>16))flush();}
};
// Mask paths are intentionally not selected; this ABI preserves the convex
// RLE dispatch used by the measured >=32-device-pixel-wide source fixtures.
struct MaskAdditiveBlitter:AdditiveBlitter {SkAlpha*getRow(int){return nullptr;}};
