// Freestanding <pthread.h> shim: no-op mutex surface for zita's
// resampler-table.h, the ONLY pthread consumer in the gxfx link chain —
// Resampler_mutex guards the static Resampler_table singleton's
// create/destroy (grep-verified; nothing else in the chain touches
// pthreads). The mixer module is single-threaded by design, and this is
// exactly what emscripten provides without -pthread, which is why the
// OctOBX wasm build never hit this.
//
// Static-ctor note: Resampler_table has a class-static Resampler_mutex
// (_mutex, resampler-table.cc) with a constructor, so this TU may carry
// an .init_array entry running it. With these no-ops inlined at -O2 the
// ctor body is empty (init does nothing), so whether LLVM elides the
// entry or an empty ctor simply never runs is semantically identical:
// the mutex ops are no-ops and zita's table creation is driven by
// explicit create() calls, not the ctor. Anything beyond a no-op ctor
// here would be a stop-and-report condition. [AUDITED: the wasm object
// carries NO .init_array at all — LLVM elided the no-op ctor entirely
// and _mutex is plain zero-initialized .bss.]
#pragma once

typedef int pthread_mutex_t; // trivial placeholder — never dereferenced

extern "C" {
inline int pthread_mutex_init(pthread_mutex_t*, const void*) { return 0; }
inline int pthread_mutex_destroy(pthread_mutex_t*) { return 0; }
inline int pthread_mutex_lock(pthread_mutex_t*) { return 0; }
inline int pthread_mutex_unlock(pthread_mutex_t*) { return 0; }
}
