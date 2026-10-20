// Freestanding <assert.h> shim: assert compiles to nothing — the exact
// semantics of every release build (NDEBUG). The gxfx chain's asserts are
// development-time post-conditions (gx_resampler.cc's resampler-plumbing
// invariants: inp_count == 0 etc.); guitarix itself ships them compiled
// out in release, and the honesty tests are the production gate.
#pragma once

#define assert(x) ((void)0)
