// gxfx/gxtuner_pthread_shim.h — CakeMix pthread neutralizer for the gxfx
// tuner TU, ported from OctOBX wasm/obxd/gxfx_shims/gxtuner_pthread_shim.h.
// This is NOT an include-order shadow of <pthread.h> (that breaks libc++:
// gxfx_prelude.h's <algorithm> chain pulls <atomic> → __thread/support/
// pthread.h, which needs the REAL pthread types) — it is included
// EXPLICITLY, once, by gxfx/gxfx_dsp_tuner.cpp, and only macro-overrides
// the three calls gx_pitch_tracker.cpp's thread machinery actually makes:
//
//   pthread_create — RECORD, don't spawn: stores the entry (always
//       PitchTracker::static_run) + argument (the tracker `this`) in
//       TU-local statics and reports success. The tuner wrapper captures
//       the pair per instance right after set_samplerate (which is what
//       triggers PitchTracker's one-time start_thread) and re-invokes the
//       entry on its ~100 ms analysis cadence; the sem_wait gate macro in
//       gxfx/semaphore.h makes each run() invocation analyze once.
//   pthread_cancel / pthread_join — no-ops returning 0 (the "handle" is
//       the fake 1 written by the recorder; a real join would dereference
//       it).
//
// Everything else (attr setup, pthread_setcancelstate, sched_get_priority_
// max in tuner.cc's init under _POSIX_PRIORITY_SCHEDULING) stays REAL —
// pure userspace struct manipulation / plain returns in this
// single-threaded build, and the values they compute are only ever passed
// to the no-op'd calls. On the freestanding wasm target the sched block is
// compiled out entirely (gxfx/shim/unistd.h does not define the feature
// macro).
//
// Freestanding wasm difference from OctOBX (which leaned on emscripten's
// REAL <pthread.h> for the types): our gxfx/shim/pthread.h provides only
// zita's no-op mutex surface, so the __wasm__ block below declares the
// thread types + no-op attr API gx_pitch_tracker.{h,cpp} reference.
// Natively the system <pthread.h> (glibc) supplies all of it and the
// block is skipped — the types can never be declared twice.
#pragma once

#ifndef GXFX_GXTUNER_PTHREAD_SHIM_H
#define GXFX_GXTUNER_PTHREAD_SHIM_H

#ifdef __cplusplus

#include <pthread.h> // REAL header: system pthread.h natively, the
                     // mutex-only gxfx/shim/pthread.h on freestanding wasm

#ifdef __wasm__
// Freestanding-only type/attr surface (glibc provides these natively; the
// existing gxfx/shim/pthread.h — zita's mutex consumer — stays untouched).
typedef unsigned long pthread_t;      // opaque handle; recorder writes fake 1
typedef struct { int __attr; } pthread_attr_t;
struct sched_param { int sched_priority; };

#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CANCEL_ENABLE   0
#define PTHREAD_SCOPE_SYSTEM    0
#define PTHREAD_EXPLICIT_SCHED  0

inline int pthread_attr_init(pthread_attr_t*) { return 0; }
inline int pthread_attr_destroy(pthread_attr_t*) { return 0; }
inline int pthread_attr_setdetachstate(pthread_attr_t*, int) { return 0; }
inline int pthread_attr_setschedpolicy(pthread_attr_t*, int) { return 0; }
inline int pthread_attr_setschedparam(pthread_attr_t*, const struct sched_param*) { return 0; }
inline int pthread_attr_setscope(pthread_attr_t*, int) { return 0; }
inline int pthread_attr_setinheritsched(pthread_attr_t*, int) { return 0; }
inline int pthread_setcancelstate(int, int*) { return 0; }
#endif // __wasm__

// Recorded "thread" — the tuner wrapper drives it cooperatively. The
// entry keeps pthread_create's exact `void* (*)(void*)` signature: wasm
// indirect calls are signature-checked, so calling through a mistyped
// function pointer traps ("null function or function signature mismatch").
static void* (*gxfx_pthread_entry)(void*) = 0;
static void* gxfx_pthread_arg = 0;

static inline int gxfx_pthread_create(pthread_t* t, const pthread_attr_t*,
                                      void* (*start_routine)(void*), void* arg) {
    gxfx_pthread_entry = start_routine;
    gxfx_pthread_arg = arg;
    if (t) *t = (pthread_t)1; // mark "created" so PitchTracker never re-spawns
    return 0;                 // success — without ever creating a thread
}

static inline int gxfx_pthread_cancel(pthread_t) { return 0; }
static inline int gxfx_pthread_join(pthread_t, void**) { return 0; }

#define pthread_create gxfx_pthread_create
#define pthread_cancel gxfx_pthread_cancel
#define pthread_join   gxfx_pthread_join

#else
#error "gxfx/gxtuner_pthread_shim.h is C++-only (the gxfx TUs are C++)"
#endif // __cplusplus

#endif // GXFX_GXTUNER_PTHREAD_SHIM_H
