// gxfx_dsp_distortion.cpp — distortion (gxfx catalog id 2): the guitarix
// bossds1 (BOSS DS-1 style) faust class as freestanding object code for
// the CakeMix mixer engine (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp.cpp (the
// authoritative wrapper TU); the resampler chain includes and the
// PortIndex enum below are transcribed from it VERBATIM (paths adapted
// to this repo's layout). bossds1 oversamples via
// gx_resample::FixedRateResampler, so — exactly as in OctOBX — this TU
// also pulls the real resampler implementation link-complete into
// itself: gx_resampler.cc plus zita's resampler-table.cc and
// resampler.cc (zita-resampler-1.1.0 bundled in the guitarix
// submodule). A second TU must never pull those (duplicated zita
// symbols).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the zita new/
// delete[] calls here resolve against them at archive link time); this
// TU defines NONE of them, no extern "C", no emscripten.
//
// Statelessness: as with the eleven TU, all param storage and strip
// state live on the Rust side; this is only the DSP class + one factory.

#include "gxfx_prelude.h"
#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP

// bossds1.cc (gx_distortion) uses gx_resample::FixedRateResampler for
// oversampling. Pull the real implementation into this TU so it is
// link-complete: gx_resampler.cc + zita resampler.cc + resampler-table.cc
// (zita-resampler-1.1.0 is bundled in the guitarix submodule).
#include "gx_resampler.h" // via -I vendor/guitarix/trunk/src/LV2/DSP
#include "gx_resampler.cc"
#include "../../../vendor/guitarix/trunk/src/zita-resampler-1.1.0/resampler-table.cc"
#include "../../../vendor/guitarix/trunk/src/zita-resampler-1.1.0/resampler.cc"

namespace gxfx_distortion {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, LEVEL, TONE, DRIVE, BYPASS } PortIndex;
#include "bossds1.cc" // via -I vendor/guitarix/trunk/src/LV2/faust-generated
}

// Factory (OctOBX naming; C++ linkage — gxfx_dsp_eleven.cpp's factory
// table is the consumer).
PluginLV2* gxfx_create_distortion() { return gxfx_distortion::bossds1::plugin(); }
