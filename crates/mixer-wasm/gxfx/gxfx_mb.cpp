// gxfx_mb.cpp — the multiband + utility guitarix family as freestanding
// object code for the CakeMix mixer engine (crates/mixer-wasm): catalog
// ids 66..70 (multiband) + 72..74 (utility).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_mb.cpp (the
// authoritative wrapper TU; paths adapted to this repo's layout — the
// guitarix sources are pulled by RELATIVE include from the pinned
// vendor/guitarix submodule, so build.rs needs no new include dirs).
// Each class is pulled into its own namespace together with its
// PortIndex enum, transcribed VERBATIM from OctOBX — which transcribed
// them from the gx_<fx>.lv2 bundle headers (== the ttl lv2:index space)
// for bundles, or the trailing PortIndex comment in the .cc itself for
// orphans. (The .cc trailing PortIndex comments of the faust multiband
// classes are alphabetized generator artifacts and are NOT the port
// order; the enums below are proven in production.) A wrong enum
// silently routes params to the wrong controls.
//
// Effect coverage (gxfx_catalog.h row order, cross-checked against the
// OctOBX src/gxfx-params.ts manifest — no keepPorts notes there; the
// enums are the port maps):
//   66 mbcompressor   67 mbdelay        68 mbdistortion   69 mbecho
//   70 barkgraphiceq  72 balance        73 outputlevel    74 ampout
// id 71 bigmuffpi is NOT here — its .cc needs gx_resample/zita,
// link-complete only inside gxfx_dsp_distortion.cpp's TU; id 75 ampmodul
// rides the future amps TU (valve.h table sharing). Same split as OctOBX.
//
// METER OUTPUTS: every multiband class writes its V* bar meters through
// `#define fVbargraphN (*fVbargraphN_)` pointers unconditionally inside
// compute() (barkgraphiceq likewise through fbargraph[]), and the engine's
// out-port surface stays unwired, so each factory parks its meters on
// TU-local scratch floats — the graphiceq precedent: throwaway values
// nobody reads, without which the first render would write through
// uninitialized pointers. Meters are NEVER params.
//
// barkgraphiceq is bundle-LOCAL dsp (gx_barkgraphiceq.lv2/): its .cc
// #includes bark_freq_grid.h (quoted — resolves in the bundle dir) and
// <iostream> from INSIDE its namespace, which upstream neuters by the
// gx_common.h chain having included <iostream> globally first. This TU
// reproduces that by including <iostream> at global scope BEFORE the
// namespace block (natively that is the real header; on the freestanding
// wasm target it is the deliberately-empty gxfx/shim/iostream, making
// barkgraphiceq.cc's in-namespace include a pragma-once no-op either
// way). orfanidis_eq.h (the bark-band EQ solver, `using namespace std` —
// scoped by including it inside our namespace) must be visible before
// the .cc, exactly like the LV2 wrapper's include order. orfanidis_eq.h
// needs <vector>: the shim's minimal std::vector serves on BOTH targets
// (see the include block below — natively, libstdc++'s real vector would
// drag in link-time internals the freestanding Rust build never
// provides, and the vendor code needs only the shim's surface).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the solver's
// vector allocations and the faust delay buffers resolve against them at
// archive link time); this TU defines NONE of them, no extern "C", no
// emscripten — only the namespaced wrappers and the eight global-scope
// factory functions below.
//
// Statelessness: as with the other gxfx TUs, all param storage and strip
// state live on the Rust side; the factory rows for ids 66..70 / 72..74
// in gxfx_dsp_eleven.cpp are the consumer (flipped in the wave that
// landed this family's tiered honesty tests, per the AGENTS.md rule).

#include "gxfx_prelude.h"
#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP
#ifdef __wasm__
#include <iostream> // gxfx/shim/iostream: empty — neuters the in-namespace include
#include <vector> // gxfx/shim/vector: the minimal std::vector (header note)
#else
// Native builds get the SAME neutering, minus libstdc++'s real headers:
// those compile fine but reference internals the freestanding Rust link
// never provides (-nodefaultlibs, no libstdc++) — std::ios_base_library_
// init() from <iostream>, std::__throw_length_error() from <vector>'s
// growth path. This TU is the archive's only consumer of either header,
// and the vendor code uses neither: barkgraphiceq.cc's <iostream> is a
// vestigial dsp2cc include (no stream object is ever touched) and the
// orfanidis solver needs exactly the shim's minimal vector surface.
// Pre-defining libstdc++'s include guards makes the vendor code's own
// <iostream>/<vector> includes no-ops, and the shims are pulled by
// relative path — one std::vector definition on BOTH targets (this TU
// flipped live in the wave that wired rows 66..74; until then mb.o was
// never archive-linked and the gap was invisible).
#define _GLIBCXX_IOSTREAM 1
#define _GLIBCXX_VECTOR 1
#include "shim/iostream"
#include "shim/vector"
#endif
// <vector> at GLOBAL scope BEFORE orfanidis_eq.h for the same reason:
// orfanidis_eq.h includes it from inside the namespace below, and a
// system <vector> re-opened there would declare gxfx_barkgraphiceq::std
// (any qualified std:: lookup in the header then fails). Pre-included
// here, the include guard makes the in-namespace include a no-op and
// std::vector stays the real global one (natively libstdc++, on the
// freestanding target gxfx/shim/vector) — reachable from the solver via
// its own `using namespace std`.

// NULL: barkgraphiceq.cc (vendor, unpatchable) assigns NULL; on the
// freestanding wasm target nothing defines it (upstream the gx_common.h
// chain's system headers do). Guarded so the native build — where the
// system headers already defined it — is untouched.
#ifndef NULL
#define NULL 0
#endif

// gx_mbcompressor.lv2 — 5-band compressor (the tree's 2nd-largest effect
// at 34 params). PortIndex transcribed from gx_mbcompressor.lv2/
// gx_mbcompressor.h (== the ttl lv2:index space): MODE/MAKEUP/
// MAKEUPTHRESHOLD/RATIO/ATTACK/RELEASE ×5 bands, 4 crossovers, then the
// V1..V10 meters + audio pair.
namespace gxfx_mbc {
typedef enum {
    MODE1, MODE2, MODE3, MODE4, MODE5,
    MAKEUP1, MAKEUP2, MAKEUP3, MAKEUP4, MAKEUP5,
    MAKEUPTHRESHOLD1, MAKEUPTHRESHOLD2, MAKEUPTHRESHOLD3, MAKEUPTHRESHOLD4, MAKEUPTHRESHOLD5,
    RATIO1, RATIO2, RATIO3, RATIO4, RATIO5,
    ATTACK1, ATTACK2, ATTACK3, ATTACK4, ATTACK5,
    RELEASE1, RELEASE2, RELEASE3, RELEASE4, RELEASE5,
    CROSSOVER_B1_B2, CROSSOVER_B2_B3, CROSSOVER_B3_B4, CROSSOVER_B4_B5,
    V1, V2, V3, V4, V5, V6, V7, V8, V9, V10,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/mbc.cc"

static float g_meter_scratch[10];
PluginLV2* create() {
    PluginLV2* p = mbc::plugin();
    if (p && p->connect_ports) {
        for (int i = V1; i <= V10; ++i)
            p->connect_ports((uint32_t)i, &g_meter_scratch[i - V1], p);
    }
    return p;
}
}

// gx_mbdelay.lv2 — 5-band delay. PortIndex from gx_mbdelay.lv2/gx_mbdelay.h.
namespace gxfx_mbdel {
typedef enum {
    DELAY1, DELAY2, DELAY3, DELAY4, DELAY5,
    FEEDBACK1, FEEDBACK2, FEEDBACK3, FEEDBACK4, FEEDBACK5,
    GAIN1, GAIN2, GAIN3, GAIN4, GAIN5,
    CROSSOVER_B1_B2, CROSSOVER_B2_B3, CROSSOVER_B3_B4, CROSSOVER_B4_B5,
    V1, V2, V3, V4, V5,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/mbdel.cc"

static float g_meter_scratch[5];
PluginLV2* create() {
    PluginLV2* p = mbdel::plugin();
    if (p && p->connect_ports) {
        for (int i = V1; i <= V5; ++i)
            p->connect_ports((uint32_t)i, &g_meter_scratch[i - V1], p);
    }
    return p;
}
}

// gx_mbdistortion.lv2 — 5-band distortion. PortIndex from
// gx_mbdistortion.lv2/gx_mbdistortion.h.
namespace gxfx_mbd {
typedef enum {
    DRIVE1, DRIVE2, DRIVE3, DRIVE4, DRIVE5,
    GAIN,
    OFFSET1, OFFSET2, OFFSET3, OFFSET4, OFFSET5,
    CROSSOVER_B1_B2, CROSSOVER_B2_B3, CROSSOVER_B3_B4, CROSSOVER_B4_B5,
    V1, V2, V3, V4, V5,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/mbd.cc"

static float g_meter_scratch[5];
PluginLV2* create() {
    PluginLV2* p = mbd::plugin();
    if (p && p->connect_ports) {
        for (int i = V1; i <= V5; ++i)
            p->connect_ports((uint32_t)i, &g_meter_scratch[i - V1], p);
    }
    return p;
}
}

// gx_mbecho.lv2 — 5-band echo. PortIndex from gx_mbecho.lv2/gx_mbecho.h.
namespace gxfx_mbe {
typedef enum {
    PERCENT1, PERCENT2, PERCENT3, PERCENT4, PERCENT5,
    TIME1, TIME2, TIME3, TIME4, TIME5,
    CROSSOVER_B1_B2, CROSSOVER_B2_B3, CROSSOVER_B3_B4, CROSSOVER_B4_B5,
    V1, V2, V3, V4, V5,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/mbe.cc"

static float g_meter_scratch[5];
PluginLV2* create() {
    PluginLV2* p = mbe::plugin();
    if (p && p->connect_ports) {
        for (int i = V1; i <= V5; ++i)
            p->connect_ports((uint32_t)i, &g_meter_scratch[i - V1], p);
    }
    return p;
}
}

// gx_barkgraphiceq.lv2 — 24-band bark-scale graphic EQ over Orfanidis
// butterworth EQs. Bundle-LOCAL dsp: barkgraphiceq.cc + orfanidis_eq.h +
// bark_freq_grid.h all live in the bundle dir (include order mirrors the
// LV2 wrapper: orfanidis_eq.h BEFORE the .cc). PortIndex from
// gx_barkgraphiceq.lv2/gx_barkgraphiceq.h (G1..G24 gains, V1..V24 band
// meters, audio pair) == the ttl index space.
namespace gxfx_barkgraphiceq {
#include "../../../vendor/guitarix/trunk/src/LV2/gx_barkgraphiceq.lv2/orfanidis_eq.h"
typedef enum {
    G1, G2, G3, G4, G5, G6, G7, G8, G9, G10, G11, G12,
    G13, G14, G15, G16, G17, G18, G19, G20, G21, G22, G23, G24,
    V1, V2, V3, V4, V5, V6, V7, V8, V9, V10, V11, V12,
    V13, V14, V15, V16, V17, V18, V19, V20, V21, V22, V23, V24,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_barkgraphiceq.lv2/barkgraphiceq.cc"

// Host-side adapter (Ts9Dsp/WahModelDsp precedent): barkgraphiceq's
// Dsp::init() — its set_samplerate — ends with clear_state_f(), which
// NULLS every fslider/fbargraph port pointer, and the ctor does the
// same. Any connection made BEFORE the sample rate is set is therefore
// wiped: this TU's original factory parked the V-meters inside create()
// (pre-rate, OctOBX style), gxfx_create then ran set_samplerate, the
// parking vanished and compute()'s unconditional `*fbargraph[j] = ...`
// wrote through NULL — hard SIGSEGV on BOTH targets (found by the smoke
// suite; OctOBX carries the same latent bug, its factory parking is
// equally pre-rate). The wrapper records every port connection and
// RE-APPLIES the full set after each set_samplerate/clear_state, so the
// meters stay parked on TU-local scratch and the host's param pointers
// survive (re)initialization at any point of the lifecycle.
static float g_meter_scratch[BARK_NUMBER_OF_BANDS];

class BarkDsp : public PluginLV2 {
private:
    barkgraphiceq::Dsp dsp_;
    float* ports_[BARK_NUMBER_OF_BANDS * 2]; // G1..G24, then V1..V24

    static BarkDsp* self(PluginLV2* p) { return static_cast<BarkDsp*>(p); }
    void reapply() {
        for (uint32_t i = 0; i < BARK_NUMBER_OF_BANDS * 2; ++i) {
            if (ports_[i] && dsp_.connect_ports)
                dsp_.connect_ports(i, ports_[i], &dsp_);
        }
    }

public:
    BarkDsp() : PluginLV2(), dsp_() {
        for (int i = 0; i < BARK_NUMBER_OF_BANDS; ++i) {
            ports_[i] = 0; // gains: the host connects these
            ports_[BARK_NUMBER_OF_BANDS + i] = &g_meter_scratch[i]; // meters parked
        }
        version = PLUGINLV2_VERSION;
        id = "barkgraphiceq";
        name = N_("Bark Grid EQ");
        mono_audio = compute_fwd;
        stereo_audio = 0;
        set_samplerate = init_fwd;
        activate_plugin = 0; // the wrapped class has none either
        connect_ports = connect_fwd;
        clear_state = clear_fwd;
        delete_instance = del_instance;
    }

    static void init_fwd(uint32_t rate, PluginLV2* p) {
        BarkDsp* s = self(p);
        if (s->dsp_.set_samplerate)
            s->dsp_.set_samplerate(rate, &s->dsp_);
        s->reapply(); // init()'s clear_state_f() just nulled the pointers
    }
    static void connect_fwd(uint32_t port, void* data, PluginLV2* p) {
        BarkDsp* s = self(p);
        if (port < BARK_NUMBER_OF_BANDS * 2)
            s->ports_[port] = static_cast<float*>(data);
        if (s->dsp_.connect_ports)
            s->dsp_.connect_ports(port, data, &s->dsp_);
    }
    static void clear_fwd(PluginLV2* p) {
        BarkDsp* s = self(p);
        if (s->dsp_.clear_state)
            s->dsp_.clear_state(&s->dsp_);
        s->reapply(); // clear_state_f() nulls the pointers again
    }
    static void compute_fwd(int count, float* input, float* output, PluginLV2* p) {
        BarkDsp* s = self(p);
        if (s->dsp_.mono_audio)
            s->dsp_.mono_audio(count, input, output, &s->dsp_);
        else if (output != input)
            memcpy(output, input, count * sizeof(float)); // fail-open passthrough
    }
    static void del_instance(PluginLV2* p) { delete self(p); }
};

PluginLV2* create() { return new BarkDsp(); }
}

// --- utility orphans (no .lv2 bundle; enums from the .cc PortIndex
// comments; no meter outputs — nothing to park) ---

// balance.cc — stereo balance (natively 2-in/2-out: stereo_audio only, so
// the host runs its stereo path).
namespace gxfx_balance {
typedef enum { BALANCE } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/balance.cc"
}

// gx_outputlevel.cc — master output level (dB). The _ladspa variant is
// only referenced by src/ladspa/ladspa_guitarix.cpp — the LV2/v1-era tree
// uses THIS class. Natively stereo.
namespace gxfx_outputlevel {
typedef enum { OUT_MASTER } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_outputlevel.cc"
}

// gx_ampout.cc — post-amp output level (dB), mono (dual-mono host). Same
// ladspa-variant rule as gx_outputlevel.
namespace gxfx_ampout {
typedef enum { OUT_AMP } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_ampout.cc"
}

// namespace paths: gxfx_mbc::create() etc. Factory order MUST match the
// catalog row order (ids 66..70 multiband + 72..74 utility; id 71
// bigmuffpi rides the resampler TU (gxfx_dsp_distortion.cpp here — in
// OctOBX it was gxfx_dsp.cpp), and id 75 ampmodul rides the future amps
// TU for valve.h table sharing).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_mbcompressor() { return gxfx_mbc::create(); }
PluginLV2* gxfx_create_mbdelay() { return gxfx_mbdel::create(); }
PluginLV2* gxfx_create_mbdistortion() { return gxfx_mbd::create(); }
PluginLV2* gxfx_create_mbecho() { return gxfx_mbe::create(); }
PluginLV2* gxfx_create_barkgraphiceq() { return gxfx_barkgraphiceq::create(); }
PluginLV2* gxfx_create_balance() { return gxfx_balance::balance::plugin(); }
PluginLV2* gxfx_create_outputlevel() { return gxfx_outputlevel::gx_outputlevel::plugin(); }
PluginLV2* gxfx_create_ampout() { return gxfx_ampout::gx_ampout::plugin(); }
