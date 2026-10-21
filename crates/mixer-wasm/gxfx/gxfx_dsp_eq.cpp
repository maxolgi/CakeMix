// gxfx_dsp_eq.cpp — the six eq-family guitarix faust DSP effects (gxfx
// catalog ids 28..33) as freestanding object code for the CakeMix mixer
// engine (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_eq.cpp
// (the authoritative wrapper TU). Each faust class is pulled into its own
// namespace together with its PortIndex enum, transcribed VERBATIM from
// OctOBX — the enum values encode the LV2 ttl port order of the
// gx_<fx>.lv2 bundle (graphiceq) or the faust-generated order (the
// orphans have no .lv2 bundle; enum from the trailing PortIndex comment
// in the .cc itself) and are proven in production; a wrong enum silently
// routes params to the wrong controls. The DSP sources are guitarix
// (GPLv2+) faust-generated classes consumed straight from the pinned
// vendor/guitarix submodule at trunk/src/LV2/faust-generated/, pulled
// via relative paths from this directory so build.rs needs no additional
// include dirs.
//
// graphiceq's compute() unconditionally dereferences its eleven bargraph
// (meter) port pointers via #define, and the host side only wires the
// catalog's control-input params (G1..G11), so — exactly as in OctOBX —
// the factory parks V1..V11 on TU-local scratch floats: throwaway meter
// values nobody reads, without which the first render would write
// through uninitialized pointers.
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the classes' new
// Dsp()/del_instance() calls resolve against them at archive link time);
// this TU defines NONE of them, no extern "C", no emscripten. The
// factory functions below are the flip targets for GXFX_FACTORIES rows
// 28..33 in gxfx_dsp_eleven.cpp — wiring the rows there is a separate
// task; this TU only supplies the symbols.

#include "gxfx_prelude.h"
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/gx_pluginlv2.h"

// gx_graphiceq.lv2 — 11-band graphic EQ. PortIndex transcribed from
// gx_graphiceq.lv2/gx_graphiceq.h (ttl order: G1..G11 params, V1..V11 band
// meters, then the audio pair) — note this is the ttl order, NOT the
// alphabetical order of the .cc's own trailing PortIndex comment.
namespace gxfx_graphiceq {
typedef enum {
    G1, G2, G3, G4, G5, G6, G7, G8, G9, G10, G11,
    V1, V2, V3, V4, V5, V6, V7, V8, V9, V10, V11,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/graphiceq.cc"

static float g_meter_scratch[11];
PluginLV2* create() {
    PluginLV2* p = graphiceq::plugin();
    if (p && p->connect_ports) {
        for (int i = V1; i <= V11; ++i)
            p->connect_ports((uint32_t)i, &g_meter_scratch[i - V1], p);
    }
    return p;
}
}

// --- orphans (no .lv2 bundle; param enums from the .cc PortIndex comment) ---

// selecteq.cc — 30 params: per band (31.25 Hz .. 16 kHz) quality, frequency
// and gain sliders; enum order is the faust-generated alphabetical-ish order
// (QS125 first, FS8K last), NOT band order — keep verbatim.
namespace gxfx_selecteq {
typedef enum {
    QS125, QS16K, QS1K, QS250, QS2K, QS31_25, QS4K, QS500, QS62_5, QS8K,
    FREQ125, FREQ16K, FREQ1K, FREQ250, FREQ2K, FREQ31_25, FREQ4K, FREQ500, FREQ62_5, FREQ8K,
    FS125, FS16K, FS1K, FS250, FS2K, FS31_25, FS4K, FS500, FS62_5, FS8K,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/selecteq.cc"
}

// tonecontroll.cc — 3-band EQ. Natively 2-in/2-out (stereo_audio only,
// mono_audio == 0), so the host runs it on its native stereo path.
namespace gxfx_tonecontroll {
typedef enum { BASS, MIDDLE, ON, TREBLE, SHARPER } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonecontroll.cc"
}

// moog.cc — 4-pole moog ladder filter. Also natively 2-in/2-out.
namespace gxfx_moog {
typedef enum { Q, FR } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/moog.cc"
}

// low_high_pass.cc — speaker band-pass stage (LOWFREQ/HIGHFREQ/ONOFF)
// cascaded with a low/high-pass stage (LOW_FREQ/HIGH_FREQ/ON_OFF).
namespace gxfx_low_high_pass {
typedef enum { HIGHFREQ, LOWFREQ, ONOFF, HIGH_FREQ, LOW_FREQ, ON_OFF } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/low_high_pass.cc"
}

namespace gxfx_noise_shaper {
typedef enum { SHARPER } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/noise_shaper.cc"
}

// Factories (OctOBX naming and order — ids 28..33, the generator manifest
// order; C++ linkage, gxfx_dsp_eleven.cpp's factory table is the consumer).
PluginLV2* gxfx_create_graphiceq() { return gxfx_graphiceq::create(); }
PluginLV2* gxfx_create_selecteq() { return gxfx_selecteq::selecteq::plugin(); }
PluginLV2* gxfx_create_tonecontroll() { return gxfx_tonecontroll::tonecontroll::plugin(); }
PluginLV2* gxfx_create_moog() { return gxfx_moog::moog::plugin(); }
PluginLV2* gxfx_create_low_high_pass() { return gxfx_low_high_pass::low_high_pass::plugin(); }
PluginLV2* gxfx_create_noise_shaper() { return gxfx_noise_shaper::noise_shaper::plugin(); }
