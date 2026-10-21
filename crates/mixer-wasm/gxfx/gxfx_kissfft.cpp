// gxfx_kissfft.cpp — THE single compilation owner of the vendored kissfft
// real-FFT core (gxfx/kissfft/, BSD-3 — provenance in kissfft/README.octobx)
// for the whole gxfx archive. kiss_fft.c and kiss_fftr.c are #included here
// at global scope inside extern "C" (never compiled as bare .c, never
// included by a second TU — the vendoring task's constraint): the kiss_*
// symbols keep C linkage and this TU is their one definition site, so every
// later consumer — the partitioned convolver (gxfx_convolver.cpp, pulled by
// gxfx_dsp_conv.cpp) and the future detune/tuner fftw3.h shim TUs — links
// against exactly one copy at archive resolution time.
//
// Exported symbol surface (exactly kissfft's real-FFT public API, nothing
// else; the kf_* helpers are static, kiss_fft_free/kiss_fftr_next_fast_
// size_real are macros over free/kiss_fft_next_fast_size):
//   kiss_fft_alloc  kiss_fft  kiss_fft_stride  kiss_fft_cleanup
//   kiss_fft_next_fast_size  kiss_fftr_alloc  kiss_fftr  kiss_fftri
//
// Undefined surface: malloc/free (KISS_FFT_MALLOC/KISS_FFT_FREE default to
// the C allocation pair — the archive's floor), the libm subset (floor/
// sqrt/cos/sin in the twiddle/factor code), and memcpy (kiss_fft_stride's
// in-place temp copy; fixed-size at -O2, wasm-lowered otherwise). All are
// on the established object-audit whitelist.
//
// NDEBUG is defined HERE, before the includes (build.rs has no per-file
// define mechanism; defining it in-TU is equivalent for these sources).
// NDEBUG is the ONLY thing keeping KISS_FFT_LOG_MSG — and its fprintf/
// stderr references — out of the freestanding object: without it the
// error paths (allocation failure, usage misuse) would emit fprintf
// libcalls that the wasm32-unknown-unknown link cannot resolve.
#ifndef NDEBUG
#define NDEBUG 1
#endif

extern "C" {
#include "kissfft/kiss_fft.c"
#include "kissfft/kiss_fftr.c"
}
