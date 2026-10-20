// Freestanding <math.h> shim: the libm subset at C-name scope for the
// zita + clipping.h chain (sin/cos/fabs/ceil in the window and table
// code, copysign in the clip curves, M_PI in the table sincs), the same
// extern "C" set <cmath> declares — the undefined symbols are the plain
// C names the Rust/wasm link resolves.
#pragma once
#define M_PI 3.14159265358979323846
extern "C" {
double sin(double x);
double cos(double x);
double tan(double x);
double asin(double x);
double acos(double x);
double atan(double x);
double atan2(double y, double x);
double exp(double x);
double log(double x);
double log10(double x);
double pow(double x, double y);
double sqrt(double x);
double fabs(double x);
double floor(double x);
double ceil(double x);
double fmod(double x, double y);
double copysign(double x, double y);
float sinf(float x);
float cosf(float x);
float tanf(float x);
float asinf(float x);
float acosf(float x);
float atanf(float x);
float atan2f(float y, float x);
float expf(float x);
float logf(float x);
float log10f(float x);
float powf(float x, float y);
float sqrtf(float x);
float fabsf(float x);
float floorf(float x);
float ceilf(float x);
float fmodf(float x, float y);
float copysignf(float x, float y);
}
