// gxfx_dsp_reverb.cpp — the reverb family (gxfx catalog ids 57..60) as
// freestanding object code for the CakeMix mixer engine (crates/mixer-wasm):
// zita_rev1 (Zita Reverb, stereo), freeverb (mono), room_simulator (mono),
// shimmizita (stereo).
//
// Provenance: productionized port of OctOBX
// wasm/obxd/gxfx_dsp_reverb.cpp (the authoritative wrapper TU). Each faust
// class is pulled into its own namespace together with its PortIndex enum,
// transcribed VERBATIM from OctOBX — the enum values encode the LV2 ttl
// port indexes of the gx_<fx>.lv2 bundles (zita_rev1/shimmizita/
// room_simulator) or the orphan .cc's own trailing enum comment
// (freeverb), and are proven in production; a wrong enum silently routes
// params to the wrong controls. All vendored sources are included by
// RELATIVE path into ../../../vendor/guitarix/trunk/ (OctOBX's style), so
// build.rs needs no new include dirs.
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the `new Dsp()`
// calls in these classes' plugin() resolve against them at archive link
// time); this TU defines NONE of them, no extern "C", no emscripten.
//
// MEMORY-CLASS: unlike chorus/delay/echo (lazily-new'd buffers), all four
// classes carry their delay lines as FIXED double arrays inside the Dsp
// object — shimmizita ~4.8 MB (six fVec[65536] + more), room_simulator
// ~1.9 MB, zita_rev1 ~1.8 MB, freeverb ~0.1 MB — allocated at CREATION
// (plugin()'s new), like digital_delay's fVec2. The Rust host's
// memory-class instance cap covers them; DSP create stays eager at
// message time, never inside process().
//
// Statelessness: as with the other gxfx TUs, all param storage and strip
// state live on the Rust side; this is only the DSP classes + four
// factories. The factory table in gxfx_dsp_eleven.cpp is the consumer
// (rows 57..60 flip from null in the table-wiring task).

#include "gxfx_prelude.h"
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/gx_pluginlv2.h"

// --- gx_zita_rev1.lv2 (faust-generated class) -------------------------------
// FDN reverb (8x8 feedback delay network, 2-band tilt EQ, predelay,
// dry/wet). PortIndex from gx_zita_rev1.h (= ttl indexes 0..14; control
// ports FIRST at 0..10, audio at 11..14). The .cc's own trailing enum
// comment is the stale dsp order — the wrapper enum rules. STEREO.
namespace gxfx_zita_rev1 {
typedef enum {
    LEVEL, EQ2_FREQ, EQ1_LEVEL, EQ1_FREQ, IN_DELAY, LOW_RT60, LF_X,
    HF_DAMPING, MID_RT60, DRY_WET_MIX, EQ2_LEVEL,
    EFFECTS_OUTPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT, EFFECTS_INPUT1,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_zita_rev1.cc"
}

// --- freeverb orphan (faust-generated class) --------------------------------
// Classic Schroeder/Moorer reverb (8 comb filters + 4 allpass per the
// freeverb topology). Mono — the host runs it dual-mono. PortIndex = the
// .cc's own trailing enum comment (the orphan generator's source).
namespace gxfx_freeverb {
typedef enum { ROOMSIZE, DAMP, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/freeverb.cc"
}

// --- gx_room_simulator.lv2 (bundle-local room_simulator.cc) -----------------
// Small/medium/large room simulation via faust delay networks (room size
// 0..3 crossfades the three room circuits). PortIndex from
// gx_room_simulator.h (= ttl indexes 0..6; audio FIRST, then the 5
// controls). Mono — the host runs it dual-mono.
namespace gxfx_room_simulator {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    EFFECT, PREDELAYMS, RT, ROOMSIZE, DRYWET,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_room_simulator.lv2/room_simulator.cc"
}

// --- gx_shimmizita.lv2 (bundle-local shimmizita.cc) -------------------------
// Zita-style FDN reverb with a parametric pitch-shift shimmer path
// (envelope-driven pitch shifter feeding the network; mode/shift/psdrywet
// control it). PortIndex from gx_shimmizita.h (= ttl indexes 0..15; audio
// at 0..3, then the 12 controls). STEREO.
namespace gxfx_shimmizita {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT1,
    CONTROL, DEPTH, DRYWET, ENVELOPE, F1, F2, MODE, PSDRYWET, SHIFT, SPEED,
    T60DS, T60M,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_shimmizita.lv2/shimmizita.cc"
}

// Factories (OctOBX naming; C++ linkage — gxfx_dsp_eleven.cpp's factory
// table is the consumer; order MUST match the manifest ids 57..60).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_zita_rev1() { return gxfx_zita_rev1::gx_zita_rev1::plugin(); }
PluginLV2* gxfx_create_freeverb() { return gxfx_freeverb::freeverb::plugin(); }
PluginLV2* gxfx_create_room_simulator() { return gxfx_room_simulator::room_simulator::plugin(); }
PluginLV2* gxfx_create_shimmizita() { return gxfx_shimmizita::shimmizita::plugin(); }
