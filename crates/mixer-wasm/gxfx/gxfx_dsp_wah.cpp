// gxfx_dsp_wah.cpp — the wah family (gxfx catalog ids 34..37: wahmodel,
// crybaby, autowah, dunwah) as freestanding object code for the CakeMix
// mixer engine (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_wah.cpp
// (the authoritative wrapper TU). Each guitarix class is pulled into its
// own namespace together with its PortIndex enum, transcribed VERBATIM
// from OctOBX — the enum values encode the LV2 ttl port indexes of the
// gx_<fx>.lv2 bundles (or the .cc's trailing faust comment for the
// orphan/bundle-local classes) and are proven in production; a wrong
// enum silently routes params to the wrong controls. All pulled sources
// are consumed straight from the pinned vendor/guitarix submodule via
// paths relative to this directory (mirroring OctOBX's relative-include
// style), so build.rs needs no new include dirs.
//
// Contents:
//  - the 7 wah models behind the "Wah Model" host-side aggregate
//    (gx_colwah.lv2: colorsound/dallas/foxx/jen/maestro/selmer/vox),
//  - the crybaby orphan (faust-generated manual Dunlop CryBaby),
//  - gxautowah.lv2's multiply-inheriting `crybaby` class driving both
//    the envelope-driven auto wah (id 36) and the manual classic wah
//    (id 37).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the new/delete
// calls in the wrapper classes below resolve against them at archive
// link time); this TU defines NONE of them, no extern "C", no
// emscripten.
//
// Statelessness: as with the other gxfx TUs, all param storage and strip
// state live on the Rust side; this is only DSP classes + factories. The
// factory-table rows for ids 34..37 in gxfx_dsp_eleven.cpp are flipped in
// the wave-integration step that owns this family's honesty tests (per
// the AGENTS.md rule: a ported effect stays rejected by set_slot until
// its known-answer test lands).

#include "gxfx_prelude.h"
#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP

// --- the 7 wah models behind the "Wah Model" aggregate (gx_colwah.lv2) ---
// gx_colwah.lv2/wah.h pulls these from faust-generated/; all seven share
// the identical param shape (WAH/FREQ/MODE/WET_DRY, class PortIndex 0..3
// from each file's trailing enum comment), so the aggregate surface is
// exactly those four + the wrapper's MODEL selector — no
// union-with-defaults needed.

namespace gxfx_wcolwah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/colwah.cc"
}

namespace gxfx_wdallaswah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/dallaswah.cc"
}

namespace gxfx_wfoxwah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/foxwah.cc"
}

namespace gxfx_wjenwah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/jenwah.cc"
}

namespace gxfx_wmaestrowah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/maestrowah.cc"
}

namespace gxfx_wselwah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/selwah.cc"
}

namespace gxfx_wvoxwah {
typedef enum { WAH, FREQ, MODE, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/voxwah.cc"
}

namespace gxfx_wahmodel {

// Model id order = the ttl MODEL scale points (gx_colwah.ttl: 0
// Colorsound, 1 Dallas, 2 Foxx, 3 Jen, 4 Maestro, 5 Selmer, 6 Vox V847);
// mirrors wah_model[] in upstream gx_colwah.cpp.
enum { WAH_MODEL_COUNT = 7 };

static PluginLV2* wah_model_factory(int model) {
    switch (model) {
    case 0: return gxfx_wcolwah::colwah::plugin();
    case 1: return gxfx_wdallaswah::dallaswah::plugin();
    case 2: return gxfx_wfoxwah::foxwah::plugin();
    case 3: return gxfx_wjenwah::jenwah::plugin();
    case 4: return gxfx_wmaestrowah::maestrowah::plugin();
    case 5: return gxfx_wselwah::selwah::plugin();
    case 6: return gxfx_wvoxwah::voxwah::plugin();
    default: return 0;
    }
}

// Host-side aggregate: ONE stable PluginLV2 surface whose MODEL param
// hot-swaps the underlying model DSP. Wahs are stateless filters, so a
// model change needs no state migration — create the new instance,
// connect the shared params, destroy the old after the swap. Only the
// ACTIVE model's instance exists at any time (upstream instead
// pre-creates all 7; one-at-a-time keeps slot cost flat). The wrapper
// PortIndex is the ttl port space (audio 0/1 + BYPASS 7 excluded —
// BYPASS is the wrapper-level enabled designation the host filters out).
class WahModelDsp : public PluginLV2 {
private:
    uint32_t rate_;
    bool active_;
    int model_;          // current underlying model, -1 = none yet
    PluginLV2* inst_;
    enum {               // gx_colwah.ttl control-port indexes
        P_WAH = 2, P_FREQ = 3, P_MODE = 4, P_MODEL = 5, P_WET_DRY = 6,
    };
    float* wah_;
    float* freq_;
    float* mode_;
    float* model_ptr_;
    float* wet_dry_;

    void connect_model_params(PluginLV2* p) {
        if (!p->connect_ports) return;
        if (wah_) p->connect_ports(0, wah_, p);      // class port WAH
        if (freq_) p->connect_ports(1, freq_, p);    // class port FREQ
        if (mode_) p->connect_ports(2, mode_, p);    // class port MODE
        if (wet_dry_) p->connect_ports(3, wet_dry_, p); // class port WET_DRY
    }
    static void destroy_inst(PluginLV2* p) {
        if (!p) return;
        if (p->activate_plugin) p->activate_plugin(false, p);
        if (p->delete_instance) p->delete_instance(p);
    }
    void ensure_model(int m) {
        if (m == model_ && inst_) return;
        PluginLV2* nu = wah_model_factory(m);
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
    WahModelDsp()
        : PluginLV2(), rate_(48000), active_(false), model_(-1), inst_(0),
          wah_(0), freq_(0), mode_(0), model_ptr_(0), wet_dry_(0) {
        version = PLUGINLV2_VERSION;
        id = "wahmodel";
        name = N_("Wah Model");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
    ~WahModelDsp() { destroy_inst(inst_); }

    static void init_static(uint32_t rate, PluginLV2* p) {
        WahModelDsp* s = static_cast<WahModelDsp*>(p);
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
        WahModelDsp* s = static_cast<WahModelDsp*>(p);
        float* f = static_cast<float*>(data);
        // forward to the live instance too, so the host's post-move
        // re-connect re-points the underlying DSP as well
        PluginLV2* inst = s->inst_;
        switch (port) {
        case P_WAH:
            s->wah_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(0, f, inst);
            break;
        case P_FREQ:
            s->freq_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(1, f, inst);
            break;
        case P_MODE:
            s->mode_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(2, f, inst);
            break;
        case P_MODEL:
            s->model_ptr_ = f;
            break;
        case P_WET_DRY:
            s->wet_dry_ = f;
            if (inst && inst->connect_ports) inst->connect_ports(3, f, inst);
            break;
        default:
            break;
        }
    }
    static int activate_static(bool start, PluginLV2* p) {
        WahModelDsp* s = static_cast<WahModelDsp*>(p);
        s->active_ = start;
        if (s->inst_ && s->inst_->activate_plugin) s->inst_->activate_plugin(start, s->inst_);
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        WahModelDsp* s = static_cast<WahModelDsp*>(p);
        if (s->inst_ && s->inst_->clear_state) s->inst_->clear_state(s->inst_);
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        WahModelDsp* s = static_cast<WahModelDsp*>(p);
        int m = 0;
        if (s->model_ptr_) { // integer param; round + clamp defensively
            m = (int)(*s->model_ptr_ + 0.5f);
            if (m < 0) m = 0;
            if (m > WAH_MODEL_COUNT - 1) m = WAH_MODEL_COUNT - 1;
        }
        if (m != s->model_ || !s->inst_) s->ensure_model(m);
        if (s->inst_ && s->inst_->mono_audio)
            s->inst_->mono_audio(count, input, output, s->inst_);
        else if (output != input)
            memcpy(output, input, count * sizeof(float)); // fail-open passthrough
    }
    static void del_instance(PluginLV2* p) { delete static_cast<WahModelDsp*>(p); }
};

PluginLV2* create() { return new WahModelDsp(); }

} // namespace gxfx_wahmodel

// --- crybaby orphan (faust-generated, manual Dunlop CryBaby) ---
// No .lv2 bundle; param enum + metadata from the .cc's trailing
// PortIndex comment + connect_ports comments.

namespace gxfx_crybaby {
typedef enum { LEVEL, WAH, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/crybaby.cc"
}

// --- gxautowah.lv2 (bundle-local dunwahauto.cc) ---------------------------
// The bundle ships TWO plugin variants over one class: `crybaby`
// multiply inherits the envelope-driven `dunwahauto` (run) and the
// manual `dunwah` (run_d); the LV2 wrapper picks the method by plugin
// URI (#autowah vs #wah — same trick here via a member-function
// pointer). PortIndex transcribed from gxautowah.lv2/gxautowah.h;
// BYPASS (port 2) is the wrapper-level enabled designation — never
// connected here. The auto variant has no control params at all (0-param
// menu entry); the manual variant only WAH.

namespace gxfx_gxautowah {
typedef enum { AUTOWAH_OUTPUT, AUTOWAH_INPUT, BYPASS, WAH } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gxautowah.lv2/dunwahauto.cc"

typedef void (crybaby::*run_fn)(uint32_t, float*, float*);

class GxAutoWahDsp : public PluginLV2 {
private:
    crybaby wah_;
    run_fn run_;
    bool auto_;

public:
    GxAutoWahDsp(bool is_auto)
        : PluginLV2(), wah_(), run_(0), auto_(is_auto) {
        // separate assignments: &crybaby::run / &crybaby::run_d have the
        // base-class member-pointer types (dunwahauto / dunwah) and cannot
        // unify in one expression — both convert to the crybaby type.
        run_ = is_auto ? &crybaby::run
                       : static_cast<run_fn>(&crybaby::run_d);
        version = PLUGINLV2_VERSION;
        id = is_auto ? "autowah" : "dunwah";
        name = N_(is_auto ? "Auto Wah" : "Classic Wah");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }

    static void init_static(uint32_t rate, PluginLV2* p) {
        GxAutoWahDsp* s = static_cast<GxAutoWahDsp*>(p);
        if (s->auto_) s->wah_.init(rate);
        else s->wah_.init_d(rate);
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        GxAutoWahDsp* s = static_cast<GxAutoWahDsp*>(p);
        // connect_d switches on PortIndex itself (only case WAH: connects)
        if (!s->auto_) s->wah_.connect_d(port, data);
    }
    static int activate_static(bool start, PluginLV2* p) {
        // upstream activate_f(): state clear on the way in, no-op out
        if (start) clear_static(p);
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        GxAutoWahDsp* s = static_cast<GxAutoWahDsp*>(p);
        if (s->auto_) s->wah_.clear_state_f();
        else s->wah_.clear_state_fd();
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        GxAutoWahDsp* s = static_cast<GxAutoWahDsp*>(p);
        (s->wah_.*s->run_)((uint32_t)count, input, output);
    }
    static void del_instance(PluginLV2* p) { delete static_cast<GxAutoWahDsp*>(p); }
};

PluginLV2* create_auto() { return new GxAutoWahDsp(true); }
PluginLV2* create_manual() { return new GxAutoWahDsp(false); }

} // namespace gxfx_gxautowah

// Factory functions (OctOBX naming; C++ linkage — gxfx_dsp_eleven.cpp's
// factory table is the consumer). Factory order MUST match the catalog
// id order 34..37 (wahmodel, crybaby, autowah, dunwah).
PluginLV2* gxfx_create_wahmodel() { return gxfx_wahmodel::create(); }
PluginLV2* gxfx_create_crybaby() { return gxfx_crybaby::crybaby::plugin(); }
PluginLV2* gxfx_create_autowah() { return gxfx_gxautowah::create_auto(); }
PluginLV2* gxfx_create_dunwah() { return gxfx_gxautowah::create_manual(); }
