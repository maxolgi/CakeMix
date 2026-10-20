#pragma once
// Minimal headless stand-in for guitarix gx_common.h + gx_compiler.h
// (adapted unchanged in substance from OctOBX wasm/obxd/gxfx_prelude.h):
// no SSE fxsave denormal control, no custom section attributes, no lv2
// headers. On the freestanding wasm32 target these standard headers
// resolve to the gxfx/shim/ dir (kept FIRST on the include path);
// natively the system libc/libstdc++ headers serve unchanged — so the
// <unistd.h> include stays: only the shim provides it there.
#include <cstdint>
#include <cstdlib>
#include <cmath>
#include <algorithm>
#include <cstring>
#include <unistd.h>
// Unqualified min/max at global scope: gx_vibe.lv2/vibe.cc's lfo namespaces
// call min(192000, max(1, rate)) bare (upstream they resolve through the
// gx_common.h include chain we do not pull). <algorithm> is already included
// above; these using-declarations expose std::min/std::max globally for all
// gxfx TUs.
using std::min;
using std::max;
#define FAUSTFLOAT float
#ifndef N_
#define N_(String) (String)
#endif
#define always_inline inline __attribute__((always_inline))
#define __rt_func
#define __rt_data
// faustpower helpers (verbatim semantics from gx_common.h, global scope)
template <int32_t N> inline float faustpower(float x) { return powf(x, N); }
template <int32_t N> inline double faustpower(double x) { return pow(x, N); }
template <int32_t N> inline int32_t faustpower(int32_t x) { return faustpower<N/2>(x) * faustpower<N-N/2>(x); }
template <> inline int32_t faustpower<0>(int32_t x) { return 1; }
template <> inline int32_t faustpower<1>(int32_t x) { return x; }
template<class T> inline T mydsp_faustpower2_f(T x) {return (x * x);}
template<class T> inline T mydsp_faustpower3_f(T x) {return ((x * x) * x);}
template<class T> inline T mydsp_faustpower4_f(T x) {return (((x * x) * x) * x);}
template<class T> inline T mydsp_faustpower5_f(T x) {return ((((x * x) * x) * x) * x);}
template<class T> inline T mydsp_faustpower6_f(T x) {return (((((x * x) * x) * x) * x) * x);}
template<class T> inline T mydsp_faustpower7_f(T x) {return x * x * x * x * x * x * x;}
