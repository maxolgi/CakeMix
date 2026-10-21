// gxfx/semaphore.h — CakeMix stand-in for <semaphore.h> inside the gxfx
// tuner TU, ported from OctOBX wasm/obxd/gxfx_shims/semaphore.h.
// gx_pitch_tracker.h includes <semaphore.h> and holds a sem_t member;
// upstream, add() sem_posts the analysis thread which sem_waits inside
// run()'s infinite loop.
//
// Inline-analysis strategy (no threads in the mixer):
//   - sem_init/sem_post are plain no-ops — arming/copying still happens
//     in add(), the "wake" is the wrapper's own cadence.
//   - sem_wait is a MACRO GATE that RETURNS FROM THE ENCLOSING FUNCTION on
//     every call after the first: each invocation of PitchTracker::run()
//     then performs EXACTLY ONE analysis pass and returns (first sem_wait
//     passes through into the analysis body; when the for(;;) loops back
//     to sem_wait, the gate returns). The wrapper resets the TU-local
//     counter (gxfx_sem_reset) right before each run() invocation.
//
// The gate counter lives here as a static (TU-local, header-only pattern
// of the fftw3.h shim); gxfx_dsp_tuner.cpp is the only TU that ever sees
// this header (only gx_pitch_tracker.h includes <semaphore.h>).
//
// Resolution: gx_pitch_tracker.h's angle-include must find THIS file on
// BOTH targets. gxfx/ is on every include list (the freestanding wasm
// build searches gxfx/shim first, which has no semaphore.h, then gxfx/;
// native builds search gxfx/ before the system dirs), so the vendored
// include resolves here everywhere and the gate semantics are identical
// on both targets — natively glibc's real <semaphore.h> must NEVER win:
// a real sem_wait would block forever inside run()'s for(;;).
//
// sem_t: int on the freestanding target. Natively glibc's <pthread.h>
// chain may already have defined sem_t (guarded __sem_t_defined); in that
// case the existing type is reused — the no-op init/post and the gate
// never dereference it either way.
#pragma once

#ifndef GXFX_SEMAPHORE_H
#define GXFX_SEMAPHORE_H

#ifdef __cplusplus

static unsigned gxfx_sem_gate = 0;

// Wrapper call: reset the gate so the NEXT sem_wait passes through.
static inline void gxfx_sem_reset(void) { gxfx_sem_gate = 0; }

#ifndef __sem_t_defined
typedef int sem_t;
#endif

static inline int sem_init(sem_t*, int, unsigned) { return 0; }
static inline int sem_post(sem_t*) { return 0; }

// The gate: first call since gxfx_sem_reset() falls through (analysis
// runs); any later call returns from the enclosing function (run() exits).
#define sem_wait(x) do { if (++gxfx_sem_gate > 1) return; } while (0)

#else
#error "gxfx/semaphore.h is C++-only (the gxfx TUs are C++)"
#endif // __cplusplus

#endif // GXFX_SEMAPHORE_H
