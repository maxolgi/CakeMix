// gxfx_dsp_mod.cpp — the modulation family's second wave (gxfx catalog
// ids 38..44) as freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_mod.cpp
// (the authoritative wrapper TU). Every namespace, PortIndex enum and
// factory function name is transcribed VERBATIM from OctOBX — the enum
// values encode the LV2 ttl port indexes of the gx_<fx>.lv2 bundles
// (the faust files' own trailing PortIndex comments are stale
// pre-dsp2cc orderings and are deliberately NOT used; for vibe the
// WRAPPER space from gx_vibe.h is the ttl port space) and a wrong enum
// silently routes params to the wrong controls. Only the include paths
// were adapted to this repo's layout (relative pulls into
// vendor/guitarix, mirroring OctOBX's relative-include style, so
// build.rs needs no new include dirs). Catalog ids covered (checked
// against gxfx_catalog.h == OctOBX src/gxfx-params.ts manifest rows
// 38..44):
//   38 vibe, 39 tubetremelo, 40 tubevibrato, 41 switched_tremolo,
//   42 phaser_st (Classic Phaser), 43 flanger_st (Classic Flanger),
//   44 chorus_mono. detune/tuner (ids 80/81) are NOT here — separate
//   wave with their own shim.
//
// 12ax7 tables: gxtubetremelo.cc / gxtubevibrato.cc each #include the
// four DSP/circuit_tables/ 12ax7 headers by bare quoted name, and
// circuit_tables is deliberately off build.rs's include set (same
// policy as tube_tables) — the four input/output12ax7*_table.h redirect
// shims in this directory (new one-line files, same mechanism as the
// w20_*/alembic_* shims, but deliberately UNGUARDED) make those includes
// resolve. Unlike the fuzz family's guard-carrying trany.h these headers
// carry NO include guard, so the global-scope include-once trick cannot
// share them — each namespace below embeds its own copy of the tables
// (~32 KB static float data per namespace), mirroring upstream's
// one-copy-per-bundle layout (the data itself stays the vendor
// submodule's single copy; a guard on the shims would neuter the second
// tube namespace's tables).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the classes'
// new/delete calls here resolve against them at archive link time); this
// TU defines NONE of them, no extern "C", no emscripten.
//
// Vibe zeroed-construction (adapted from OctOBX, same defect, different
// mechanics): vibe.cc's Vibe ctor leaves its filter-state members
// (vc/vcvo/ecvc/vevo .x1/.y1, fbl/fbr, oldstepl/...) UNINITIALIZED —
// upstream rides on hosts instantiating onto effectively fresh memory.
// Our slot model news/deletes DSP instances onto a recycled wasm heap,
// and a reused chunk can carry leftover float garbage into those states
// (observed in OctOBX as NaN output on the very first rendered block,
// surviving no param change — the state feeds back into itself), so the
// instance must be constructed over zeroed memory. OctOBX pairs
// ::operator new + memset + placement new; this TU may define neither
// operator new nor memset, and an UNDEFINED memset reference is outside
// this archive's object-audit contract, so the zeroing is a volatile
// byte loop instead (volatile stores are also immune to the compiler's
// memset loop-idiom formation — probed on both targets) and the raw
// block comes from malloc, pairing exactly with the deleter's delete,
// which routes to eleven's global operator delete -> free().
//
// Known statics quirk (faithful to upstream): vibe.cc's LFO phase
// accumulators AND its TEMPO/DF slider pointers live in the file-static
// vibe_lfo_sine / vibe_mono_lfo_sine namespaces, shared by every Vibe
// instance in the process — two Vibe slots share one LFO phase and the
// last-connected slot's TEMPO wins (upstream LV2 hosts loading the .so
// twice behave identically).
//
// Statelessness: as with the eleven/distortion TUs, all param storage
// and strip state live on the Rust side; this is only the DSP classes +
// factories. Ids 38..44 stay unavailable (null factories in
// gxfx_dsp_eleven.cpp's table) until a follow-up task flips those rows —
// this TU enters the archive unlinked until then.

#include "gxfx_prelude.h"
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/gx_pluginlv2.h"

// C-scope libm ahead of the vendor pulls, plus two wasm-only gaps in the
// shim math.h the pulled sources ride:
//  - signbit: gxtubetremelo.cc / gxtubevibrato.cc call it bare (their
//    own '#include "math.h"' sits at file scope, so natively the
//    libstdc++ wrapper hoists 'using std::signbit;' to global scope; the
//    freestanding shim declares no signbit at all). Declared here over
//    the clang builtins — folded inline, no libcall, no state.
//  - M_E: vibe.cc computes with CNST_E == M_E; the shim math.h carries
//    M_PI but not M_E. Guarded so the native define (libstdc++ pulls
//    _GNU_SOURCE) is never redefined.
#include <math.h>
#ifdef __wasm__
inline bool signbit(double x) { return __builtin_signbit(x); }
inline bool signbit(float x) { return __builtin_signbitf(x); }
// std::abs is the one std::-qualified call the shim <cmath> subset has
// no alias for (vibe.cc: fcross = 1 - std::abs(flrcross)) — emscripten's
// libc++ provided it in OctOBX. Overloads over the clang builtins,
// folded inline; same adaptation class as the signbit pair above.
namespace std {
inline double abs(double x) { return __builtin_fabs(x); }
inline float abs(float x) { return __builtin_fabsf(x); }
} // namespace std
#endif
#ifndef M_E
#define M_E 2.7182818284590452354
#endif

// Standard placement new: DECLARED only (this TU defines no allocation
// functions — eleven owns the replaceable globals). The declaration is
// needed because the freestanding shim set has no <new>; the form is
// the non-replaceable standard placement allocation function, and at
// build.rs's -O2 the call folds to nothing on both targets (probed: no
// _ZnwYPv reference is emitted, native or wasm). Global size_t (the
// shim set typedefs it at global scope only, no std::size_t).
void* operator new(size_t, void*) noexcept;

// --- gx_vibe.lv2 (bundle-local vibe.cc) ------------------------------------
// Uni-Vibe-style photocell modulator (Ryan Billing, GPL2+). The file ships
// THREE namespaces (vibe_mono_lfo_sine, vibe_lfo_sine, vibe) — all nested
// under one gxfx_vibe wrapper namespace here. PortIndex is the WRAPPER
// space from gx_vibe.h (= the ttl port indexes); the class's own trailing
// PortIndex comment is stale (pre-dsp2cc ordering) and unused because
// gx_vibe.cpp defines the type before including vibe.cc — our typedef
// plays that role. We instantiate plugin_stereo() (ttl declares in/in1);
// the plugin_mono() variant stays unshipped (dual-mono of the stereo
// class covers the mono use case badly — the stereo LFO phase offset is
// half the vibe sound — so we deliberately ship the real stereo one
// only).
namespace gxfx_vibe {
typedef enum {
    WIDTH, DEPTH, WETDRY, FB, TEMPO,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    DF, PAN, CROSS,
    EFFECTS_OUTPUT1, EFFECTS_INPUT1,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_vibe.lv2/vibe.cc"

// Construct over zeroed memory (see the TU header): malloc'd block,
// volatile byte-zeroed, placement-constructed. Returns 0 on allocation
// failure — the eleven-TU ABI treats a null plugin as a failed create.
inline PluginLV2* plugin_stereo_zeroed() {
    void* raw = malloc(sizeof(vibe::Vibe));
    if (!raw)
        return 0;
    volatile unsigned char* p = static_cast<volatile unsigned char*>(raw);
    for (size_t i = 0; i < sizeof(vibe::Vibe); ++i)
        p[i] = 0;
    return new (raw) vibe::Vibe(true);
}
} // namespace gxfx_vibe

// --- gxtubetremelo.lv2 / gxtubevibrato.lv2 (faust-generated classes) -------
// 12ax7 tube-modelled tremolo / vibrato (vactrol model by "transmogrify").
// PortIndex transcribed from the bundles' gxtubetremelo.h /
// gxtubevibrato.h (= the ttl port space; the .cc trailing comments carry
// a STALE enum order from the .dsp — the ttl/h order is what the shipped
// wrapper uses).

namespace gxfx_tubetremelo {
typedef enum { SINEWAVE, DEPTH, SPEED, DRIVE, OUTPUT, EFFECTS_OUTPUT, EFFECTS_INPUT } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxtubetremelo.cc"
}

namespace gxfx_tubevibrato {
typedef enum { SINEWAVE, DEPTH, SPEED, DRIVE, OUTPUT, EFFECTS_OUTPUT, EFFECTS_INPUT } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxtubevibrato.cc"
}

// --- gx_switched_tremolo.lv2 (faust-generated class) ------------------------
// Stepped tremolo that hops between up to 4 settable frequencies.
// PortIndex transcribed from gx_switched_tremolo.h (audio out/in first,
// then DEPTH..WET_DRY at ttl indexes 2..9).

namespace gxfx_switched_tremolo {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, DEPTH, FREQ0, FREQ1, FREQ2, FREQ3, STEPS, SWITCHFREQ, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/switched_tremolo.cc"
}

// --- classic faust orphans ---------------------------------------------------
// phaser.cc (10 params, natively 2-in/2-out — v1's id 6 "Phaser" is the
// leaner phaser_mono.cc class) and flanger.cc (7 params, natively stereo —
// v1's id 5 "Flanger" is the gx_flanger.cc class). chorus_mono.cc is the
// mono sibling of v1's id 4 "Chorus" (chorus.cc, stereo).

namespace gxfx_phaser_st {
typedef enum { MAXNOTCH1FREQ, MINNOTCH1FREQ, NOTCHWIDTH, NOTCHFREQ, SPEED, VIBRATOMODE, DEPTH, FEEDBACKGAIN, INVERT, LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/phaser.cc"
}

namespace gxfx_flanger_st {
typedef enum { LFOFREQ, DEPTH, FEEDBACKGAIN, DELAY, DELAYOFFSET, INVERT, LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/flanger.cc"
}

namespace gxfx_chorus_mono {
typedef enum { FREQ, LEVEL, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/chorus_mono.cc"
}

// namespace paths: gxfx_vibe::vibe::plugin_stereo_zeroed() etc. Factory
// order MUST match the manifest order (ids 38..44). C++ linkage —
// gxfx_dsp_eleven.cpp's factory table is the consumer (rows flip in a
// follow-up).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_vibe() { return gxfx_vibe::plugin_stereo_zeroed(); }
PluginLV2* gxfx_create_tubetremelo() { return gxfx_tubetremelo::gxtubetremelo::plugin(); }
PluginLV2* gxfx_create_tubevibrato() { return gxfx_tubevibrato::gxtubevibrato::plugin(); }
PluginLV2* gxfx_create_switched_tremolo() { return gxfx_switched_tremolo::switched_tremolo::plugin(); }
PluginLV2* gxfx_create_phaser_st() { return gxfx_phaser_st::phaser::plugin(); }
PluginLV2* gxfx_create_flanger_st() { return gxfx_flanger_st::flanger::plugin(); }
PluginLV2* gxfx_create_chorus_mono() { return gxfx_chorus_mono::chorus_mono::plugin(); }
