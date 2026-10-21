// gxfx_dsp_amps.cpp — the guitarix amp + tonestack family (plus preamps
// and the ampmodul/metal postamps) as freestanding object code for the
// CakeMix mixer engine (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_amps.cpp
// (the authoritative wrapper TU). Every namespace, PortIndex enum and
// factory function name is transcribed VERBATIM from OctOBX — the enums
// encode the LV2 ttl port indexes of the gx_amp.lv2 wrapper space the
// classes' connect() switches expect, and the amp/tonestack model orders
// mirror upstream gx_amp.lv2/gxamp.cpp amp_model[] / tonestack_model[]
// (amp ids == the gx_amp.ttl `model` scale points 0..18, gxnoamp at 18;
// tonestack 0..26 in tonestack_model[] order, i.e. the gx_amp.ttl
// `t_model` space minus the historical "Off" hole at 26). Only the
// include paths were adapted to this repo's layout (relative pulls into
// vendor/guitarix, mirroring OctOBX's relative-include style, so
// build.rs needs no new include dirs). Catalog ids covered (checked
// against gxfx_catalog.h == OctOBX src/gxfx-params.ts manifest):
//   61 ampmodel, 62 tonestack, 63 studiopre (stereo variant),
//   64 alembic, 65 w20, 75 ampmodul; gxfx_ampsdsp_metalamp/metalhead are
//   NOT host factories — the Phase 2-b cab wrappers (gxfx_dsp_conv.cpp
//   in OctOBX) call them for their preamp stage, catalog ids 78/79.
//
// valve.h INCLUDE-ONCE TRICK: every gxampN.cc starts with
// `#include "valve.h"`, which pulls the 14 DSP/tube_tables/*.cc files
// (~2.2 MB of plain float arrays) plus the tubetab/tubetab2 index arrays
// and the inline Ftube()/Ranode() interpolators. valve.h carries an
// include guard — including it at GLOBAL scope HERE, BEFORE the
// namespace blocks, neuters every gxampN's own include: the tables +
// index arrays are defined ONCE and shared by all 19 amp namespaces
// (each namespace still reaches ::Ftube / ::tubetab via unqualified
// lookup). NOTE: in this repo `#include "valve.h"` resolves to the
// gxfx/valve.h shim (this directory precedes the vendor DSP dir on the
// -I chain) because the vendor valve.h's own bare tube-table includes
// would not resolve without a tube_tables include dir; the shim is a
// verbatim transcription with repo-relative table includes. gxmetal_*
// and gx_ampmodul ride the same guard. This is the ONLY TU in the
// archive that may include valve.h (the tube table globals have
// external linkage; a second TU would be a duplicate-symbol link error
// — gxfx_dsp_distortion.cpp's trany.h discipline is the analogue).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the new/delete
// calls here resolve against them at archive link time); this TU defines
// NONE of them, no extern "C", no emscripten.
//
// Statelessness: as with the eleven/distortion TUs, all param storage,
// chaining and strip state live on the Rust side; the aggregates below
// (AmpModelDsp / ToneStackModelDsp, verbatim from OctOBX) exist purely
// to hot-swap the per-model plugin instances behind one MODEL param
// (create new + connect + activate-if-active + destroy old, re-checked
// every block). The alembic/w20 circuit tables are `static` per the
// vendor headers, so the gxfx/ redirect shims keep them
// namespace-internal — only ONE studiopre variant (the stereo one,
// gx_studiopre_st.cc, the catalog's stereo:1 row 63) can live in this
// TU because both studiopre .cc files include the guard-less alembic
// tables.

#include "gxfx_prelude.h"
#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP

// signbit at GLOBAL scope: gx_studiopre_st.cc and gx_w20.cc call it bare,
// and the first of the two also #includes "math.h" from INSIDE its faust
// namespace — libstdc++'s math.h wrapper would then hoist
// `using std::signbit;` into that namespace, leaving the sibling
// namespaces without it (emscripten's math.h makes signbit a macro, which
// is why OctOBX never hit this). Including math.h HERE first pins the
// using-declarations to global scope, and the wrapper's own guard neuters
// the in-namespace copy.
#include <math.h>
#ifdef __wasm__
// The freestanding shim math.h carries no signbit: declare it over the
// clang builtins (folded inline, no libcall, no state) for the two faust
// files above.
inline bool signbit(double x) { return __builtin_signbit(x); }
inline bool signbit(float x) { return __builtin_signbitf(x); }
#endif

// --- valve.h at GLOBAL scope (see header): tables defined once, shared ----
#include "valve.h" // gxfx/ shim — see the file's header comment

// --- the 19 amp model classes (gxamp.cc, gxamp2..18.cc, gxnoamp.cc) -------
// Per-namespace PortIndex = the gxamp.h wrapper values the classes' connect
// switches expect (GAIN1=0 PREGAIN=1 WET_DRY=2 DRIVE=3 ... HIGHGAIN=18).

namespace gxfx_gxamp {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp.cc"
}

namespace gxfx_gxamp2 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp2.cc"
}

namespace gxfx_gxamp3 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp3.cc"
}

namespace gxfx_gxamp4 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp4.cc"
}

namespace gxfx_gxamp5 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp5.cc"
}

namespace gxfx_gxamp6 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp6.cc"
}

namespace gxfx_gxamp7 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp7.cc"
}

namespace gxfx_gxamp8 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp8.cc"
}

namespace gxfx_gxamp9 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp9.cc"
}

namespace gxfx_gxamp10 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp10.cc"
}

namespace gxfx_gxamp11 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp11.cc"
}

namespace gxfx_gxamp12 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp12.cc"
}

namespace gxfx_gxamp13 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp13.cc"
}

namespace gxfx_gxamp14 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp14.cc"
}

namespace gxfx_gxamp15 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp15.cc"
}

namespace gxfx_gxamp16 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp16.cc"
}

namespace gxfx_gxamp17 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp17.cc"
}

namespace gxfx_gxamp18 {
typedef enum { GAIN1 = 0, PREGAIN = 1, WET_DRY = 2, DRIVE = 3, HIGHGAIN = 18 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxamp18.cc"
}

namespace gxfx_gxnoamp {
typedef enum { GAIN1 = 0, PREGAIN = 1 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxnoamp.cc"
}

// --- the 27 STEREO tonestack classes ---------------------------------------
// Per-namespace PortIndex = the gxamp.h wrapper values (MIDDLE=4 BASS=5
// TREBLE=6) the classes' connect switches expect. Every tonestack model has
// a _stereo class in faust-generated/, so the aggregate runs native stereo.

namespace gxfx_ts_default {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_default_stereo.cc"
}

namespace gxfx_ts_bassman {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_bassman_stereo.cc"
}

namespace gxfx_ts_twin {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_twin_stereo.cc"
}

namespace gxfx_ts_princeton {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_princeton_stereo.cc"
}

namespace gxfx_ts_jcm800 {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_jcm800_stereo.cc"
}

namespace gxfx_ts_jcm2000 {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_jcm2000_stereo.cc"
}

namespace gxfx_ts_mlead {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_mlead_stereo.cc"
}

namespace gxfx_ts_m2199 {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_m2199_stereo.cc"
}

namespace gxfx_ts_ac30 {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_ac30_stereo.cc"
}

namespace gxfx_ts_soldano {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_soldano_stereo.cc"
}

namespace gxfx_ts_mesa {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_mesa_stereo.cc"
}

namespace gxfx_ts_jtm45 {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_jtm45_stereo.cc"
}

namespace gxfx_ts_ac15 {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_ac15_stereo.cc"
}

namespace gxfx_ts_peavey {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_peavey_stereo.cc"
}

namespace gxfx_ts_ibanez {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_ibanez_stereo.cc"
}

namespace gxfx_ts_roland {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_roland_stereo.cc"
}

namespace gxfx_ts_ampeg {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_ampeg_stereo.cc"
}

namespace gxfx_ts_ampeg_rev {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_ampeg_rev_stereo.cc"
}

namespace gxfx_ts_sovtek {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_sovtek_stereo.cc"
}

namespace gxfx_ts_bogner {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_bogner_stereo.cc"
}

namespace gxfx_ts_groove {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_groove_stereo.cc"
}

namespace gxfx_ts_crunch {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_crunch_stereo.cc"
}

namespace gxfx_ts_fender_blues {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_fender_blues_stereo.cc"
}

namespace gxfx_ts_fender_default {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_fender_default_stereo.cc"
}

namespace gxfx_ts_fender_deville {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_fender_deville_stereo.cc"
}

namespace gxfx_ts_gibsen {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_gibsen_stereo.cc"
}

namespace gxfx_ts_engl {
typedef enum { MIDDLE = 4, BASS = 5, TREBLE = 6 } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/tonestack_engl_stereo.cc"
}

namespace gxfx_ampmodel {

// Model id order = the gx_amp.ttl `model` scale points == upstream
// amp_model[] in gx_amp.lv2/gxamp.cpp (18 tube combos + gxnoamp "--" at 18).
enum { AMP_MODEL_COUNT = 19 };

static PluginLV2* amp_model_factory(int model) {
    switch (model) {
    case 0: return gxfx_gxamp::gxamp::plugin();
    case 1: return gxfx_gxamp3::gxamp3::plugin();
    case 2: return gxfx_gxamp14::gxamp14::plugin();
    case 3: return gxfx_gxamp10::gxamp10::plugin();
    case 4: return gxfx_gxamp18::gxamp18::plugin();
    case 5: return gxfx_gxamp2::gxamp2::plugin();
    case 6: return gxfx_gxamp9::gxamp9::plugin();
    case 7: return gxfx_gxamp11::gxamp11::plugin();
    case 8: return gxfx_gxamp17::gxamp17::plugin();
    case 9: return gxfx_gxamp13::gxamp13::plugin();
    case 10: return gxfx_gxamp5::gxamp5::plugin();
    case 11: return gxfx_gxamp4::gxamp4::plugin();
    case 12: return gxfx_gxamp15::gxamp15::plugin();
    case 13: return gxfx_gxamp12::gxamp12::plugin();
    case 14: return gxfx_gxamp7::gxamp7::plugin();
    case 15: return gxfx_gxamp8::gxamp8::plugin();
    case 16: return gxfx_gxamp16::gxamp16::plugin();
    case 17: return gxfx_gxamp6::gxamp6::plugin();
    case 18: return gxfx_gxnoamp::gxnoamp::plugin();
    default: return 0;
    }
}

// Host-side aggregate over the 19 amp classes (WahModelDsp lifecycle: one
// live instance, MODEL re-read each block, hot swap = create + connect +
// activate-if-active + destroy old). Port space = the gx_amp.ttl / gxamp.h
// wrapper indexes (audio 15/16, tonestack 4..6/10 and cab 7/8/11 ports are
// NOT part of this entry — the aggregate exposes the amp-model params only,
// per fx2plan: the wrapper-level tonestack + convolver cab stages stay out;
// tonestacks have their own entry, cabs are Phase 2).
class AmpModelDsp : public PluginLV2 {
private:
    uint32_t rate_;
    bool active_;
    int model_;          // current underlying model, -1 = none yet
    PluginLV2* inst_;
    enum {               // gx_amp.ttl control-port indexes (gxamp.h space)
        P_GAIN1 = 0, P_PREGAIN = 1, P_WET_DRY = 2, P_DRIVE = 3,
        P_MODEL = 9, P_HIGHGAIN = 18,
    };
    float* gain1_;
    float* pregain_;
    float* wet_dry_;
    float* drive_;
    float* model_ptr_;
    float* highgain_;

    void connect_model_params(PluginLV2* p) {
        if (!p->connect_ports) return;
        if (gain1_) p->connect_ports(P_GAIN1, gain1_, p);
        if (pregain_) p->connect_ports(P_PREGAIN, pregain_, p);
        if (wet_dry_) p->connect_ports(P_WET_DRY, wet_dry_, p);
        if (drive_) p->connect_ports(P_DRIVE, drive_, p);
        if (highgain_) p->connect_ports(P_HIGHGAIN, highgain_, p);
    }
    static void destroy_inst(PluginLV2* p) {
        if (!p) return;
        if (p->activate_plugin) p->activate_plugin(false, p);
        if (p->delete_instance) p->delete_instance(p);
    }
    void ensure_model(int m) {
        if (m == model_ && inst_) return;
        PluginLV2* nu = amp_model_factory(m);
        if (!nu) return; // allocation failure: keep the old instance running
        if (nu->set_samplerate) nu->set_samplerate(rate_, nu);
        connect_model_params(nu);
        if (active_ && nu->activate_plugin) nu->activate_plugin(true, nu);
        PluginLV2* old = inst_;
        inst_ = nu;
        model_ = m;
        destroy_inst(old); // destroy AFTER the swap
    }

public:
    AmpModelDsp()
        : PluginLV2(), rate_(48000), active_(false), model_(-1), inst_(0),
          gain1_(0), pregain_(0), wet_dry_(0), drive_(0), model_ptr_(0), highgain_(0) {
        version = PLUGINLV2_VERSION;
        id = "ampmodel";
        name = N_("Amp Model");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
    ~AmpModelDsp() { destroy_inst(inst_); }

    static void init_static(uint32_t rate, PluginLV2* p) {
        AmpModelDsp* s = static_cast<AmpModelDsp*>(p);
        s->rate_ = rate;
        if (s->inst_) {
            if (s->inst_->set_samplerate) s->inst_->set_samplerate(rate, s->inst_);
        } else {
            // eager-create the default model (0) — set_samplerate runs at
            // slot-assign time (a worklet task), so the first render after
            // enable never allocates; later swaps ride the MODEL param.
            s->ensure_model(0);
        }
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        AmpModelDsp* s = static_cast<AmpModelDsp*>(p);
        float* f = static_cast<float*>(data);
        // forward to the live instance too, so the host's post-move
        // re-connect (fx_move_slot) re-points the underlying DSP as well
        PluginLV2* inst = s->inst_;
        switch (port) {
        case P_GAIN1:
            s->gain1_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_GAIN1, f, inst);
            break;
        case P_PREGAIN:
            s->pregain_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_PREGAIN, f, inst);
            break;
        case P_WET_DRY:
            s->wet_dry_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_WET_DRY, f, inst);
            break;
        case P_DRIVE:
            s->drive_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_DRIVE, f, inst);
            break;
        case P_MODEL:
            s->model_ptr_ = f;
            break;
        case P_HIGHGAIN:
            s->highgain_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_HIGHGAIN, f, inst);
            break;
        default:
            break;
        }
    }
    static int activate_static(bool start, PluginLV2* p) {
        AmpModelDsp* s = static_cast<AmpModelDsp*>(p);
        s->active_ = start;
        if (s->inst_ && s->inst_->activate_plugin) s->inst_->activate_plugin(start, s->inst_);
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        AmpModelDsp* s = static_cast<AmpModelDsp*>(p);
        if (s->inst_ && s->inst_->clear_state) s->inst_->clear_state(s->inst_);
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        AmpModelDsp* s = static_cast<AmpModelDsp*>(p);
        int m = 0;
        if (s->model_ptr_) { // integer param; round + clamp defensively
            m = (int)(*s->model_ptr_ + 0.5f);
            if (m < 0) m = 0;
            if (m > AMP_MODEL_COUNT - 1) m = AMP_MODEL_COUNT - 1;
        }
        if (m != s->model_ || !s->inst_) s->ensure_model(m);
        if (s->inst_ && s->inst_->mono_audio)
            s->inst_->mono_audio(count, input, output, s->inst_);
        else if (output != input)
            memcpy(output, input, count * sizeof(float)); // fail-open passthrough
    }
    static void del_instance(PluginLV2* p) { delete static_cast<AmpModelDsp*>(p); }
};

PluginLV2* create() { return new AmpModelDsp(); }

} // namespace gxfx_ampmodel

namespace gxfx_tonemodel {

// Model id order = upstream tonestack_model[] in gx_amp.lv2/gxamp.cpp ==
// the gx_amp.ttl `t_model` scale points with the historical "Off" hole at
// 26 removed (ttl 27 "Engl" -> here 26).
enum { TS_MODEL_COUNT = 27 };

static PluginLV2* tonestack_model_factory(int model) {
    switch (model) {
    case 0: return gxfx_ts_default::tonestack_default_stereo::plugin();
    case 1: return gxfx_ts_bassman::tonestack_bassman_stereo::plugin();
    case 2: return gxfx_ts_twin::tonestack_twin_stereo::plugin();
    case 3: return gxfx_ts_princeton::tonestack_princeton_stereo::plugin();
    case 4: return gxfx_ts_jcm800::tonestack_jcm800_stereo::plugin();
    case 5: return gxfx_ts_jcm2000::tonestack_jcm2000_stereo::plugin();
    case 6: return gxfx_ts_mlead::tonestack_mlead_stereo::plugin();
    case 7: return gxfx_ts_m2199::tonestack_m2199_stereo::plugin();
    case 8: return gxfx_ts_ac30::tonestack_ac30_stereo::plugin();
    case 9: return gxfx_ts_soldano::tonestack_soldano_stereo::plugin();
    case 10: return gxfx_ts_mesa::tonestack_mesa_stereo::plugin();
    case 11: return gxfx_ts_jtm45::tonestack_jtm45_stereo::plugin();
    case 12: return gxfx_ts_ac15::tonestack_ac15_stereo::plugin();
    case 13: return gxfx_ts_peavey::tonestack_peavey_stereo::plugin();
    case 14: return gxfx_ts_ibanez::tonestack_ibanez_stereo::plugin();
    case 15: return gxfx_ts_roland::tonestack_roland_stereo::plugin();
    case 16: return gxfx_ts_ampeg::tonestack_ampeg_stereo::plugin();
    case 17: return gxfx_ts_ampeg_rev::tonestack_ampeg_rev_stereo::plugin();
    case 18: return gxfx_ts_sovtek::tonestack_sovtek_stereo::plugin();
    case 19: return gxfx_ts_bogner::tonestack_bogner_stereo::plugin();
    case 20: return gxfx_ts_groove::tonestack_groove_stereo::plugin();
    case 21: return gxfx_ts_crunch::tonestack_crunch_stereo::plugin();
    case 22: return gxfx_ts_fender_blues::tonestack_fender_blues_stereo::plugin();
    case 23: return gxfx_ts_fender_default::tonestack_fender_default_stereo::plugin();
    case 24: return gxfx_ts_fender_deville::tonestack_fender_deville_stereo::plugin();
    case 25: return gxfx_ts_gibsen::tonestack_gibsen_stereo::plugin();
    case 26: return gxfx_ts_engl::tonestack_engl_stereo::plugin();
    default: return 0;
    }
}

// Host-side aggregate over the 27 STEREO tonestack classes (WahModelDsp
// lifecycle; the underlying classes are stateless biquad stacks, so a model
// change needs no state migration). Port space = the gx_amp.ttl / gxamp.h
// wrapper indexes the classes' connect switches expect (MIDDLE=4 BASS=5
// TREBLE=6; MODEL parked at t_model's 10).
class ToneStackModelDsp : public PluginLV2 {
private:
    uint32_t rate_;
    bool active_;
    int model_;          // current underlying model, -1 = none yet
    PluginLV2* inst_;
    enum {               // gx_amp.ttl control-port indexes (gxamp.h space)
        P_MIDDLE = 4, P_BASS = 5, P_TREBLE = 6, P_MODEL = 10,
    };
    float* middle_;
    float* bass_;
    float* treble_;
    float* model_ptr_;

    void connect_model_params(PluginLV2* p) {
        if (!p->connect_ports) return;
        if (middle_) p->connect_ports(P_MIDDLE, middle_, p);
        if (bass_) p->connect_ports(P_BASS, bass_, p);
        if (treble_) p->connect_ports(P_TREBLE, treble_, p);
    }
    static void destroy_inst(PluginLV2* p) {
        if (!p) return;
        if (p->activate_plugin) p->activate_plugin(false, p);
        if (p->delete_instance) p->delete_instance(p);
    }
    void ensure_model(int m) {
        if (m == model_ && inst_) return;
        PluginLV2* nu = tonestack_model_factory(m);
        if (!nu) return; // allocation failure: keep the old instance running
        if (nu->set_samplerate) nu->set_samplerate(rate_, nu);
        connect_model_params(nu);
        if (active_ && nu->activate_plugin) nu->activate_plugin(true, nu);
        PluginLV2* old = inst_;
        inst_ = nu;
        model_ = m;
        destroy_inst(old); // destroy AFTER the swap
    }

public:
    ToneStackModelDsp()
        : PluginLV2(), rate_(48000), active_(false), model_(-1), inst_(0),
          middle_(0), bass_(0), treble_(0), model_ptr_(0) {
        version = PLUGINLV2_VERSION;
        id = "tonestack";
        name = N_("Tone Stack");
        mono_audio = 0;
        stereo_audio = compute_static;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
    ~ToneStackModelDsp() { destroy_inst(inst_); }

    static void init_static(uint32_t rate, PluginLV2* p) {
        ToneStackModelDsp* s = static_cast<ToneStackModelDsp*>(p);
        s->rate_ = rate;
        if (s->inst_) {
            if (s->inst_->set_samplerate) s->inst_->set_samplerate(rate, s->inst_);
        } else {
            // eager-create the default model (0) — see AmpModelDsp
            s->ensure_model(0);
        }
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        ToneStackModelDsp* s = static_cast<ToneStackModelDsp*>(p);
        float* f = static_cast<float*>(data);
        // forward to the live instance too (fx_move_slot re-connect)
        PluginLV2* inst = s->inst_;
        switch (port) {
        case P_MIDDLE:
            s->middle_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_MIDDLE, f, inst);
            break;
        case P_BASS:
            s->bass_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_BASS, f, inst);
            break;
        case P_TREBLE:
            s->treble_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(P_TREBLE, f, inst);
            break;
        case P_MODEL:
            s->model_ptr_ = f;
            break;
        default:
            break;
        }
    }
    static int activate_static(bool start, PluginLV2* p) {
        ToneStackModelDsp* s = static_cast<ToneStackModelDsp*>(p);
        s->active_ = start;
        if (s->inst_ && s->inst_->activate_plugin) s->inst_->activate_plugin(start, s->inst_);
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        ToneStackModelDsp* s = static_cast<ToneStackModelDsp*>(p);
        if (s->inst_ && s->inst_->clear_state) s->inst_->clear_state(s->inst_);
    }
    static void compute_static(int count, float* input1, float* input2,
                               float* output1, float* output2, PluginLV2* p) {
        ToneStackModelDsp* s = static_cast<ToneStackModelDsp*>(p);
        int m = 0;
        if (s->model_ptr_) { // integer param; round + clamp defensively
            m = (int)(*s->model_ptr_ + 0.5f);
            if (m < 0) m = 0;
            if (m > TS_MODEL_COUNT - 1) m = TS_MODEL_COUNT - 1;
        }
        if (m != s->model_ || !s->inst_) s->ensure_model(m);
        if (s->inst_ && s->inst_->stereo_audio)
            s->inst_->stereo_audio(count, input1, input2, output1, output2, s->inst_);
        else {
            if (output1 != input1) memcpy(output1, input1, count * sizeof(float));
            if (output2 != input2) memcpy(output2, input2, count * sizeof(float));
        }
    }
    static void del_instance(PluginLV2* p) { delete static_cast<ToneStackModelDsp*>(p); }
};

PluginLV2* create() { return new ToneStackModelDsp(); }

} // namespace gxfx_tonemodel

// --- preamps ----------------------------------------------------------------

// gx_studiopre_st: STEREO variant of the Studio preamp (two 12AX7 sections
// + tonestack + volume per channel). PortIndex from gx_studiopre_st.h = the
// ttl indexes 0..11 (the .cc's own trailing enum comment is the stale
// alphabetical dsp order). Ships INSTEAD of the mono gx_studiopre.cc — both
// include the guard-less alembic_* circuit tables (here via the gxfx/
// redirect shims, namespace-internal since the vendor tables are static),
// so only one can live in this TU; the stereo class is the plan's
// preference (catalog row 63 is stereo).
namespace gxfx_studiopre_st {
typedef enum {
    BRIGHT_L, VOLUME_L, BASS_L, MIDDLE_L, TREBLE_L, MASTER_L,
    BRIGHT_R, VOLUME_R, BASS_R, MIDDLE_R, TREBLE_R, MASTER_R,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_studiopre_st.cc"
}

// gx_alembic: Alembic-style bass preamp (no tables — self-contained).
// PortIndex = the ttl indexes 0..4 (== the .cc's trailing enum).
namespace gxfx_alembic {
typedef enum { AUDIO_IN, BASS, MIDDLE, TREBLE, VOLUME } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_alembic.cc"
}

// gx_w20: W20 preamp (tiltdrivepro-style circuit tables embedded in its own
// namespace — w20_1/w20_2a headers from DSP/circuit_tables/, here via the
// gxfx/ redirect shims). PortIndex = the ttl indexes 0..1 (== the .cc's
// trailing enum).
namespace gxfx_w20 {
typedef enum { GAIN, LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_w20.cc"
}

// gx_ampmodul ("Postamp" — Phase 1-h UTILITY family, menu-wise; the DSP is
// a modulation-adjacent tube postamp: two feedback paths around gxamp2's
// tubec, dry + wet). It lives in THIS TU, not gxfx_mb.cpp, because its .cc
// #includes "valve.h" (6V6 tables): the table symbols are non-static
// globals already defined once here at global scope, and a second TU
// including valve.h would be a duplicate-symbol link error. The include
// guard neuters the in-namespace include — same trick as the gxampN files.
// PortIndex = the .cc's trailing enum (FEEDBAC dry-path feedback, FEEDBACK
// wet-path feedback, LEVEL, HIGHGAIN, TUBE1/TUBE2 stage gains, WET_DRY).
namespace gxfx_ampmodul {
typedef enum { FEEDBAC, FEEDBACK, LEVEL, HIGHGAIN, TUBE1, TUBE2, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_ampmodul.cc"
}

// gxmetal_amp / gxmetal_head (Phase 2-b metal preamps): the faust classes
// behind the gxmetal_*.lv2 wrappers. They also #include "valve.h" (vestigial
// — the classes reference none of its symbols; dsp2cc artifact), so they ride
// THIS TU exactly like gx_ampmodul above: the global-scope valve.h include
// neuters their own, tables stay single-copy. The wrapper-level cab stage
// (fixed 4x12 IR convolution) is NOT here — gxfx_dsp_conv.cpp wraps these
// factories with the self-written convolver (fx2plan Phase 2). PortIndex =
// the bundles' gxmetal_{amp,head}.h = the ttl indexes (HIGHGAIN at 6 is
// notOnGUI → never connected; the class default 0 holds).
namespace gxfx_metalamp {
typedef enum {
    TONE, DRIVE, PREGAIN, GAIN1,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    HIGHGAIN,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxmetal_amp.cc"
}

namespace gxfx_metalhead {
typedef enum {
    TONE, DRIVE, PREGAIN, GAIN1,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    HIGHGAIN,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gxmetal_head.cc"
}

// namespace paths: gxfx_ampmodel::create() etc. Factory order MUST match the
// catalog row order (ids 61..65 + the utility ampmodul at 75 — note
// OctOBX's TU header comment says "74", stale: both the generator manifest
// and this repo's gxfx_catalog.h have ampout at 74, ampmodul at 75).
// gxfx_ampsdsp_metalamp/metalhead are NOT host factories — the Phase 2-b
// wrappers in gxfx_dsp_conv.cpp call them for their preamp stage (catalog
// ids 78/79 stay unavailable until that wrapper lands).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_ampmodel() { return gxfx_ampmodel::create(); }
PluginLV2* gxfx_create_tonemodel() { return gxfx_tonemodel::create(); }
PluginLV2* gxfx_create_studiopre() { return gxfx_studiopre_st::gx_studiopre_st::plugin(); }
PluginLV2* gxfx_create_alembic() { return gxfx_alembic::gx_alembic::plugin(); }
PluginLV2* gxfx_create_w20() { return gxfx_w20::gx_w20::plugin(); }
PluginLV2* gxfx_create_ampmodul() { return gxfx_ampmodul::gx_ampmodul::plugin(); }
PluginLV2* gxfx_ampsdsp_metalamp() { return gxfx_metalamp::gxmetal_amp::plugin(); }
PluginLV2* gxfx_ampsdsp_metalhead() { return gxfx_metalhead::gxmetal_head::plugin(); }
