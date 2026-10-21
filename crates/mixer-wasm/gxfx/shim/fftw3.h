// gxfx/shim/fftw3.h — CakeMix stand-in for FFTW3's single-precision API,
// implemented on the vendored kissfft (gxfx/kissfft/, BSD-3), ported from
// OctOBX wasm/obxd/gxfx_shims/fftw3.h (no fftw, no pthreads).
//
// Scope: ONLY the complex 1-D DFT subset detune.cc references:
//   fftwf_complex          — float[2] ([k][0]=re, [k][1]=im; same layout as
//                            kiss_fft_cpx)
//   fftwf_plan             — opaque handle; plan struct new/delete'd here
//   fftwf_plan_dft_1d(n, in, out, FFTW_FORWARD|FFTW_BACKWARD, flags)
//   fftwf_execute(plan)    — runs on the buffers bound at plan creation
//   fftwf_destroy_plan(p)
//   constants FFTW_FORWARD(-1) / FFTW_BACKWARD(+1) / FFTW_MEASURE(0)
// plus the r2r halfcomplex subset gx_pitch_tracker.cpp references (tuner):
//   fftwf_plan_r2r_1d(n, in, out, FFTW_R2HC|FFTW_HC2R, flags) — n MUST be
//                            even (kiss_fftr requirement; the tracker's
//                            2048+1025-literal nets to 3072)
//   fftwf_execute_r2r(plan, in, out) — one-shot on caller buffers
//   fftwf_execute(plan)    — dispatches r2r plans too (the tracker uses
//                            bound-buffer fftwf_execute exclusively)
//   fftwf_malloc/fftwf_free — the tracker's ffttwBufferTime/Freq arrays
//   constants FFTW_ESTIMATE / FFTW_R2HC / FFTW_HC2R
// Both r2r kinds are UNSCALED like fftw: R2HC followed by HC2R returns
// n x the input. Semantics over kissfft: the halfcomplex layout is
// out[k]=Re(X[k]) for k=0..n/2 and out[n-k]=Im(X[k]) for k=1..n/2-1 —
// exactly the indexing gx_pitch_tracker.cpp's power-spectrum loop assumes
// (|X[k]|^2 = out[k]^2 + out[n-k]^2). R2HC runs kiss_fftr (forward real
// FFT, n/2+1 complex bins) then packs the bins; HC2R unpacks into n/2+1
// bins then runs kiss_fftri (unscaled inverse real FFT). Both directions
// stage through a plan-owned bins array, so in==out (or partial aliasing)
// is safe: the input is fully consumed before the output is written.
//
// Scaling: like fftw, BOTH directions are unscaled — kiss_fft's inverse is
// unscaled too, so the shim applies no normalization (detune's hanningd
// window already carries the smbPitchShift 2/((N/2)*osamp) term; a 1/N
// here would double-scale the synthesis path — and the tracker divides the
// HC2R output by n itself, so a normalized inverse would corrupt its NSDF).
//
// Plans are NOT thread-safe — irrelevant: the mixer is single-threaded by
// design (no pthreads anywhere in the gxfx chain; the tuner's analysis
// "thread" is re-invoked inline by gxfx_dsp_tuner.cpp).
//
// kissfft linkage: gxfx_kissfft.cpp is the archive's SOLE compilation owner
// of kiss_fft.c/kiss_fftr.c (extern "C"); this header pulls only the
// kissfft DECLARATIONS and links against those symbols — never #include
// the .c files here (duplicate definitions).
//
// Header-only by design: every function is `static inline`, so any number
// of TUs can include the shim without link collisions (each TU gets its
// own copy; plan handles never cross TU boundaries).
//
// Resolution: on the freestanding wasm target gxfx/shim/ is FIRST on the
// include path, so `#include <fftw3.h>` (gx_pitch_tracker.h) resolves here
// directly; native builds do not put gxfx/shim on the -I list, so the
// forwarder gxfx/fftw3.h re-exports this header from the (both-target)
// gxfx/ include dir. Guard keeps a double include a no-op everywhere.
#pragma once

#ifndef GXFX_SHIM_FFTW3_H
#define GXFX_SHIM_FFTW3_H

#include "kissfft/kiss_fft.h"  // vendored kissfft; symbols C-linked from gxfx_kissfft.cpp
#include "kissfft/kiss_fftr.h" // real-FFT half wrapper (also compiled in gxfx_kissfft.cpp)
#include <stdlib.h>

#ifdef __cplusplus

#define FFTW_FORWARD (-1)
#define FFTW_BACKWARD (+1)

// Planning flags: fftw has a whole zoo; ported code passes FFTW_MEASURE
// (detune) or FFTW_ESTIMATE (tracker) — kissfft has no planner, the values
// are accepted and ignored.
#define FFTW_MEASURE (0)
#define FFTW_ESTIMATE (1 << 6)

// r2r transform kinds — only the halfcomplex pair any ported source uses.
typedef enum {
    FFTW_R2HC = 0,
    FFTW_HC2R = 1,
} fftwf_r2r_kind;

typedef float fftwf_complex[2];

struct fftwf_plan_s {
    // complex-DFT subset (detune)
    kiss_fft_cfg cfg;   // kissfft state (factors + twiddles)
    kiss_fft_cpx* in;   // buffers bound at plan creation — fftwf_execute
    kiss_fft_cpx* out;  // reads/writes these, argument-less like fftw
    int n;
    int sign;           // FFTW_FORWARD / FFTW_BACKWARD (kept for parity)
    // r2r halfcomplex subset (tuner) — kind < 0 marks a complex plan
    int kind;                 // fftwf_r2r_kind, or -1 for complex DFT
    kiss_fftr_cfg cfg_r;      // forward cfg for R2HC, inverse cfg for HC2R
    float* rin;               // bound real buffers (fftwf_execute path)
    float* rout;
    kiss_fft_cpx* spec;       // n/2+1 bin staging (alias-safe both ways)
};
typedef struct fftwf_plan_s* fftwf_plan;

static inline fftwf_plan fftwf_plan_dft_1d(int n, fftwf_complex* in,
                                           fftwf_complex* out, int sign,
                                           unsigned /*flags*/) {
    if (n <= 0 || !in || !out) return 0;
    kiss_fft_cfg cfg = kiss_fft_alloc(n, sign != FFTW_FORWARD, 0, 0);
    if (!cfg) return 0;
    fftwf_plan p = new fftwf_plan_s;
    p->cfg = cfg;
    p->in = reinterpret_cast<kiss_fft_cpx*>(in);
    p->out = reinterpret_cast<kiss_fft_cpx*>(out);
    p->n = n;
    p->sign = sign;
    p->kind = -1;
    p->cfg_r = 0;
    p->rin = p->rout = 0;
    p->spec = 0;
    return p;
}

// One r2r transform over the caller's (or plan-bound) buffers. Both kinds
// stage through p->spec so the input is fully consumed before `out` is
// written — safe for in == out and for fftw's in-place plans.
static inline void fftwf_r2r_run(const fftwf_plan p, const float* in, float* out) {
    if (!p || !p->cfg_r || !in || !out) return;
    const int n = p->n;
    const int h = n / 2;
    if (p->kind == FFTW_R2HC) {
        kiss_fftr(p->cfg_r, in, p->spec);
        for (int k = 0; k <= h; ++k) out[k] = p->spec[k].r;
        for (int k = 1; k < h; ++k) out[n - k] = p->spec[k].i;
    } else { // FFTW_HC2R — unscaled inverse of the halfcomplex layout
        p->spec[0].r = in[0];   p->spec[0].i = 0; // imag of DC/Nyquist is
        p->spec[h].r = in[h];   p->spec[h].i = 0; // not stored (and ignored)
        for (int k = 1; k < h; ++k) {
            p->spec[k].r = in[k];
            p->spec[k].i = in[n - k];
        }
        kiss_fftri(p->cfg_r, p->spec, out);
    }
}

static inline fftwf_plan fftwf_plan_r2r_1d(int n, float* in, float* out,
                                           fftwf_r2r_kind kind, unsigned /*flags*/) {
    // kiss_fftr needs an even n (it runs a half-length complex FFT); the
    // tracker's only size is 3072. Odd requests yield a null plan, which
    // gx_pitch_tracker.cpp turns into its own error path.
    if (n <= 0 || (n & 1) || !in || !out) return 0;
    if (kind != FFTW_R2HC && kind != FFTW_HC2R) return 0;
    kiss_fftr_cfg cfg = kiss_fftr_alloc(n, kind == FFTW_HC2R, 0, 0);
    if (!cfg) return 0;
    kiss_fft_cpx* spec = new kiss_fft_cpx[n / 2 + 1];
    fftwf_plan p = new fftwf_plan_s;
    p->cfg = 0;
    p->in = p->out = 0;
    p->n = n;
    p->sign = 0;
    p->kind = (int)kind;
    p->cfg_r = cfg;
    p->rin = in;
    p->rout = out;
    p->spec = spec;
    return p;
}

static inline void fftwf_execute(const fftwf_plan p) {
    if (!p) return;
    if (p->kind >= 0) {
        fftwf_r2r_run(p, p->rin, p->rout);
        return;
    }
    if (!p->cfg) return;
    kiss_fft(p->cfg, p->in, p->out);
}

static inline void fftwf_execute_r2r(const fftwf_plan p, float* in, float* out) {
    if (!p || p->kind < 0) return; // r2r plans only (fftw would be UB here)
    fftwf_r2r_run(p, in, out);
}

static inline void fftwf_destroy_plan(fftwf_plan p) {
    if (!p) return;
    if (p->cfg) kiss_fft_free(p->cfg); // macro -> free (kiss_fft.h)
    if (p->cfg_r) kiss_fftr_free(p->cfg_r);
    delete[] p->spec;
    delete p;
}

// Tracker-only (its ctor allocates the fftwBuffer arrays via fftwf_malloc;
// alignment is irrelevant under wasm — plain malloc).
static inline void* fftwf_malloc(size_t n) { return malloc(n ? n : 1); }
static inline void fftwf_free(void* p) { free(p); }

#else
#error "gxfx/shim/fftw3.h is C++-only (the gxfx TUs are C++; a C include path does not exist)"
#endif // __cplusplus

#endif // GXFX_SHIM_FFTW3_H
