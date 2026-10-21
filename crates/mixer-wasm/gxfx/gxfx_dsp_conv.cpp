// gxfx_dsp_conv.cpp — the convolution FX family (catalog ids 76..79) as
// freestanding object code for the CakeMix mixer engine (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_conv.cpp
// (the authoritative wrapper TU). Every namespace, PortIndex enum and
// factory function name is transcribed VERBATIM from OctOBX — the enums
// encode the LV2 ttl port indexes of the gx_cabinet/gx_redeye/gxmetal
// bundles the classes' connect() switches expect. Only the include paths
// were adapted to this repo's layout (relative pulls into
// vendor/guitarix, mirroring the other family TUs, so build.rs needs no
// new include dirs — the chump circuit tables resolve via the
// DSP/circuit_tables include dir it already passes) plus the storage
// adaptation below. Catalog ids covered (gxfx_catalog.h rows 76..79):
//   76 cabinet  "Cabinet"   77 redeye   "Redeye"
//   78 metalamp "Metal Amp" 79 metalhead "Metal Head"
// Rows 76..79 in gxfx_dsp_eleven.cpp's GXFX_FACTORIES stay NULL until the
// wave-flip task lands them with the honesty tests — this TU only ships
// the C++-linkage gxfx_create_{cabinet,redeye,metalamp,metalhead}
// factories for that flip to wire.
//
// Storage adaptation (the one functional deviation from OctOBX): the
// wrapper scratch buffers (CabinetDsp base_/work_, RedeyeDsp ir_work_)
// ride the archive's C malloc/free floor instead of std::vector — the
// freestanding <vector> shim is private to gxfx_mb.cpp and the shims are
// frozen; the convolver core made the same move (gxfx_convolver.h). The
// zita BufferResampler's new[]'d outputs are copied out and delete[]'d at
// the boundary (eleven's operator delete[] floor). IR buffers are
// memory-class per the catalog (memory=1 on rows 76..79), bounded by the
// static cab tables: 68..1000 taps @48k, resampled to engine rate.
//
// Upstream gxcabinet.cpp is an LV2 worker-thread wrapper around
// gx_convolver (fftw3f + pthreads — NOT portable into the render thread,
// fx2plan Phase 2). This TU re-implements the WRAPPER semantics on the
// self-written convolver core (gxfx_convolver.{h,cpp}):
//   cab IR (cab_data.cc, static float tables @48k)
//     → resample to engine rate (gx_resample::BufferResampler, zita)
//     → bake bass/treble shelving + level into the IR via the UNMODIFIED
//       bundle-local Impf class (cabinet_impulse_former.h — the faust
//       "impulse former" upstream applies to the IR, not the audio)
//     → partitioned overlap-save convolution (gxfx_conv).
// Model 18 "Off" is dry passthrough (upstream quirk keeps the last cab —
// the label says Off, so Off it is; documented deviation). Model 17 (1x8)
// halves the level, matching upstream's adjust_1x8.
//
// kissfft (the convolver's FFT core) is compiled by gxfx_kissfft.cpp —
// this TU pulls only its public headers transitively. zita's resampler
// implementation is link-complete from gxfx_dsp_distortion.cpp; only the
// gx_resampler.h declarations ride here.
//
// cab IR tables at GLOBAL scope (before any namespace): cab_data.cc /
// cab_data_table.cc have no include guards, so they ride exactly one TU
// of the archive (valve.h include-once precedent in gxfx_dsp_amps.cpp).
// cab_data_table.cc itself #includes cab_data.cc.
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the new/delete/
// delete[] calls here resolve against them at archive link time); this TU
// defines NONE of them, no extern "C", no emscripten.
//
// Param surface = gx_cabinet.ttl control inputs: CLevel/CBass/CTreble/
// c_model (BYPASS = lv2:enabled designation and SCHEDULE = notOnGUI are
// filtered by the generator). One audio input in the ttl → mono → the
// host runs it dual-mono (dsp + dsp_r), one convolver per channel.

#include "gxfx_prelude.h"
#include "gxfx_convolver.h"
#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP
#include "gx_resampler.h" // via -I vendor/guitarix/trunk/src/LV2/DSP

// signbit at GLOBAL scope (same discipline as gxfx_dsp_amps.cpp): the three
// chump .cc files call it bare from inside their faust namespaces, and each
// also #includes "math.h" from the top of its .cc — which lands INSIDE the
// enclosing gxfx_redeye_dsp namespace here. Including <math.h> first pins
// libstdc++'s using-declarations to global scope (the wrapper's own guard
// neuters the in-namespace copy); on the freestanding target the shim
// math.h carries no signbit at all, so declare it over the clang builtins
// (folded inline, no libcall, no state).
#include <math.h>
#ifdef __wasm__
inline bool signbit(double x) { return __builtin_signbit(x); }
inline bool signbit(float x) { return __builtin_signbitf(x); }
#endif

#include "../../../vendor/guitarix/trunk/src/LV2/DSP/cab_data_table.cc"

namespace gxfx_cabinet {

// bundle-local impulse former (gx_cabinet.lv2/cabinet_impulse_former.h) —
// unmodified faust class, namespaced like every other gxfx DSP include.
#include "../../../vendor/guitarix/trunk/src/LV2/gx_cabinet.lv2/cabinet_impulse_former.h"

class CabinetDsp : public PluginLV2 {
private:
    enum {  // gx_cabinet.ttl control-port indexes
        P_CLEVEL = 0, P_CBASS = 1, P_CTREBLE = 2, P_CMODEL = 3,
    };
    enum { CAB_COUNT = 18 };  // cab_table entries; ttl model 18 = "Off"

    uint32_t rate_;
    gxfx_conv::PartitionedConvolver conv_;
    Impf impf_;
    gx_resample::BufferResampler resamp_;

    // connected param pointers (host mirror floats)
    float* clevel_;
    float* cbass_;
    float* ctreble_;
    float* cmodel_;

    // base IR (engine rate, pre-impf) cache — malloc floor
    float* base_;
    size_t base_len_;
    int base_model_;
    uint32_t base_rate_;
    // last BAKED impf input values (re-bake only on real change)
    float level_, bass_, treble_;
    int model_;
    bool have_ir_;      // a convolver IR was pushed at least once
    bool off_;          // model 18 passthrough
    float* work_;       // impf output scratch — malloc floor
    size_t work_len_;

    void load_base(int m) {
        const CabDesc& cab = *getCabEntry((uint32_t)m).data;
        free(base_);
        base_ = 0;
        base_len_ = 0;
        if (cab.ir_sr == rate_) {
            base_len_ = (size_t)cab.ir_count;
            base_ = (float*)malloc(base_len_ * sizeof(float));
            if (base_)
                memcpy(base_, cab.ir_data, base_len_ * sizeof(float));
        } else {
            int32_t olen = 0;
            // BufferResampler::process takes a mutable input (zita filters
            // in-place-ish); the cab tables are const globals — go through
            // a copy to keep third_party data unmodified. The new[]'d
            // output is copied onto our malloc floor and delete[]'d at the
            // boundary.
            float* src = (float*)malloc((size_t)cab.ir_count * sizeof(float));
            if (src) {
                memcpy(src, cab.ir_data, (size_t)cab.ir_count * sizeof(float));
                float* out = resamp_.process((int32_t)cab.ir_sr, cab.ir_count,
                                             src, (int32_t)rate_, &olen);
                free(src);
                if (out && olen > 0) {
                    base_ = (float*)malloc((size_t)olen * sizeof(float));
                    if (base_) {
                        memcpy(base_, out, (size_t)olen * sizeof(float));
                        base_len_ = (size_t)olen;
                    }
                    delete[] out;
                }
            }
        }
        base_model_ = m;
        base_rate_ = rate_;
    }

    // Detect param changes and (re)bake the IR — runs at the top of every
    // render, cheap when nothing changed (a few float compares). Rebuilds
    // are bounded (resample <= ~1k samples + <= 8 partition FFTs) and ride
    // the convolver's pending-rebuild path: applied on THIS render's first
    // process() call, same bounded-alloc policy as lazy DSP activation.
    void maybe_rebuild() {
        const float level = clevel_ ? *clevel_ : 1.0f;
        const float bass = cbass_ ? *cbass_ : 0.0f;
        const float treble = ctreble_ ? *ctreble_ : 0.0f;
        int m = 0;
        if (cmodel_) {
            m = (int)(*cmodel_ + 0.5f);
            if (m < 0) m = 0;
            if (m > CAB_COUNT) m = CAB_COUNT;
        }
        const bool model_changed = (m != model_) || !have_ir_;
        const bool tone_changed = !have_ir_
            || fabsf(level - level_) > 1e-3f
            || fabsf(bass - bass_) > 1e-3f
            || fabsf(treble - treble_) > 1e-3f;
        if (!model_changed && !tone_changed) return;

        model_ = m;
        off_ = (m >= CAB_COUNT);
        if (off_) {
            // "Off": disable the convolver entirely (dry passthrough).
            conv_.set_ir(0, 0, (double)rate_, (double)rate_);
            have_ir_ = true; // keep the baked-value latch updated
            level_ = level; bass_ = bass; treble_ = treble;
            return;
        }
        if (model_changed || base_rate_ != rate_ || base_model_ != m)
            load_base(m);
        if (!base_ || base_len_ == 0) { // resample failure: fail-open passthrough
            conv_.set_ir(0, 0, (double)rate_, (double)rate_);
            have_ir_ = true;
            level_ = level; bass_ = bass; treble_ = treble;
            return;
        }
        // Bake bass/treble/level into the IR (upstream do_work_mono():
        // impf over the IR; 1x8 gets its level halved).
        const float adjust = (m == 17) ? 0.5f : 1.0f;
        if (work_len_ < base_len_) {
            float* nu = (float*)malloc(base_len_ * sizeof(float));
            if (!nu) { // scratch failure: fail-open passthrough
                conv_.set_ir(0, 0, (double)rate_, (double)rate_);
                have_ir_ = true;
                level_ = level; bass_ = bass; treble_ = treble;
                return;
            }
            free(work_);
            work_ = nu;
            work_len_ = base_len_;
        }
        impf_.compute((int)base_len_, base_, work_, bass, treble,
                      level * adjust);
        conv_.set_ir(work_, base_len_, (double)rate_, (double)rate_);
        have_ir_ = true;
        level_ = level; bass_ = bass; treble_ = treble;
    }

public:
    CabinetDsp()
        : PluginLV2(), rate_(48000), conv_(), impf_(), resamp_(),
          clevel_(0), cbass_(0), ctreble_(0), cmodel_(0),
          base_(0), base_len_(0), base_model_(-1), base_rate_(0),
          level_(1.0f), bass_(0.0f), treble_(0.0f),
          model_(-1), have_ir_(false), off_(false),
          work_(0), work_len_(0) {
        version = PLUGINLV2_VERSION;
        id = "cabinet";
        name = N_("Cabinet");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
    ~CabinetDsp() {
        free(base_);
        free(work_);
    }

    static void init_static(uint32_t rate, PluginLV2* p) {
        CabinetDsp* s = static_cast<CabinetDsp*>(p);
        s->rate_ = rate;
        s->impf_.init(rate);           // shelving constants are rate-dependent
        s->base_rate_ = 0;             // force base-IR re-resample
        s->have_ir_ = false;           // force a bake on next render
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        CabinetDsp* s = static_cast<CabinetDsp*>(p);
        float* f = static_cast<float*>(data);
        switch (port) {
        case P_CLEVEL: s->clevel_ = f; break;
        case P_CBASS: s->cbass_ = f; break;
        case P_CTREBLE: s->ctreble_ = f; break;
        case P_CMODEL: s->cmodel_ = f; break;
        default: break;
        }
    }
    static int activate_static(bool start, PluginLV2* p) {
        CabinetDsp* s = static_cast<CabinetDsp*>(p);
        if (start) s->conv_.reset();   // fresh streaming state on enable
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        CabinetDsp* s = static_cast<CabinetDsp*>(p);
        s->conv_.reset();
        s->have_ir_ = false;
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        CabinetDsp* s = static_cast<CabinetDsp*>(p);
        s->maybe_rebuild();
        // Unconditional: process() applies a pending IR rebuild on this
        // render (parts_ becomes > 0 there) and no-ops when disabled, so
        // gating on active() would deadlock the first rebuild forever.
        s->conv_.process((size_t)count, input);
        if (output != input)
            memcpy(output, input, (size_t)count * sizeof(float));
    }
    static void del_instance(PluginLV2* p) { delete static_cast<CabinetDsp*>(p); }
};

PluginLV2* create() { return new CabinetDsp(); }

} // namespace gxfx_cabinet

// ---------------------------------------------------------------------------
// gx_redeye.lv2 — the Redeye aggregate (effect id 77)
// ---------------------------------------------------------------------------
// Upstream gxredeye.cpp is ONE wrapper exposing THREE LV2 descriptors
// (#chump / #bigchump / #vibrochump): each picks a preamp model from
// DSP/gx_redeye.h's amp_model[] AND a FIXED speaker cab (chump → 1x8 =
// cab_table 17; bigchump/vibrochump → 2x12 = cab_table 1), runs the preamp
// then convolves with that cab IR scaled by the DSP Impf impulse former at
// value 1.0 (gain = 1.0² × 0.01 = 0.01 — the tabulated cab_data IR, NOT a
// generated IR). OctOBX folds the three descriptors into ONE menu entry
// with a MODEL param 0..2 (aggregate lifecycle per fx2plan — same shape as
// gxfx_dsp_amps.cpp's AmpModelDsp): the preamp instance is hot-swapped on
// MODEL and the model's cab IR re-pushed to the convolver. The wrapper's
// noiser.cc denormal-breaker companion is skipped on SSE builds upstream
// (#ifndef __SSE__) and per fx2plan research.
//
// PortIndex = gxredeye.h (the wrapper enum shared by all three preamp
// classes = the ttl indexes); MODEL (port 10) is aggregate-only.
//
// All three .cc ride ONE enclosing namespace, mirroring upstream exactly
// (gxredeye.cpp includes gx_redeye.h — and thereby the three classes —
// inside namespace gx_redeye): the bigchump* circuit-table headers carry
// INCLUDE GUARDS (_BIGCHUMPPRE_H_ etc.) and their #includes sit at the top
// of the .cc files OUTSIDE the class namespaces, so they must land in ONE
// shared scope for both gx_bigchump and gx_vibrochump to see them. The
// redeye* tables (chump only) are guard-less but clash with nothing.
namespace gxfx_redeye_dsp {
typedef enum {
    GAIN = 0, TONE = 1, VOLUME = 2, FEEDBACK = 3,
    VIBE = 4, SPEED = 5, INTENSITY = 6, SINEWAVE = 7,
    AMP_OUTPUT = 8, AMP_INPUT = 9,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_chump.cc"
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_bigchump.cc"
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/gx_vibrochump.cc"
}

namespace gxfx_redeye {

class RedeyeDsp : public PluginLV2 {
private:
    enum { MODEL_COUNT = 3 };
    // per-model cab (cab_table indexes, from upstream set_amp_mono):
    // chump → 17 (1x8), bigchump/vibrochump → 1 (2x12)
    static const uint32_t MODEL_CAB[MODEL_COUNT];

    uint32_t rate_;
    gxfx_conv::PartitionedConvolver conv_;
    bool active_;
    int model_;          // current preamp model, -1 = none yet
    PluginLV2* inst_;
    // connected param pointers (host mirror floats), wrapper port space
    float* params_[8];   // ports 0..7 (GAIN..SINEWAVE)
    float* model_ptr_;   // port 10 (aggregate-only)
    uint32_t ir_model_;  // cab index currently pushed to the convolver
    uint32_t ir_rate_;   // engine rate the push happened at
    float* ir_work_;     // IR × 0.01 scratch — malloc floor
    size_t ir_work_len_;

    static PluginLV2* model_factory(int m) {
        switch (m) {
        case 0: return gxfx_redeye_dsp::gx_chump::plugin();
        case 1: return gxfx_redeye_dsp::gx_bigchump::plugin();
        default: return gxfx_redeye_dsp::gx_vibrochump::plugin();
        }
    }

    void connect_model_params(PluginLV2* p) {
        if (!p->connect_ports) return;
        for (int i = 0; i < 8; ++i)
            if (params_[i]) p->connect_ports((uint32_t)i, params_[i], p);
    }
    static void destroy_inst(PluginLV2* p) {
        if (!p) return;
        if (p->activate_plugin) p->activate_plugin(false, p);
        if (p->delete_instance) p->delete_instance(p);
    }
    void ensure_model(int m) {
        if (m == model_ && inst_) return;
        PluginLV2* nu = model_factory(m);
        if (!nu) return; // allocation failure: keep the old instance running
        if (nu->set_samplerate) nu->set_samplerate(rate_, nu);
        connect_model_params(nu);
        if (active_ && nu->activate_plugin) nu->activate_plugin(true, nu);
        PluginLV2* old = inst_;
        inst_ = nu;
        model_ = m;
        destroy_inst(old); // destroy AFTER the swap
    }
    // Push the model's fixed cab IR (upstream: Impf value 1.0 → ×0.01) to
    // the convolver; the convolver copies + resamples (cab tables carry
    // their own ir_sr; ir_rate tells it so) and rebuilds on the next
    // process().
    void push_cab_ir(int m) {
        const CabDesc& cab = *getCabEntry(MODEL_CAB[m]).data;
        if (ir_work_len_ < (size_t)cab.ir_count) {
            float* nu = (float*)malloc((size_t)cab.ir_count * sizeof(float));
            if (!nu) return; // allocation failure: keep the old cab running
            free(ir_work_);
            ir_work_ = nu;
            ir_work_len_ = (size_t)cab.ir_count;
        }
        const float scale = 0.01f; // Impf(1.0): value² × 0.01
        for (int32_t i = 0; i < cab.ir_count; ++i)
            ir_work_[(size_t)i] = cab.ir_data[i] * scale;
        conv_.set_ir(ir_work_, (size_t)cab.ir_count,
                     (double)cab.ir_sr, (double)rate_);
        ir_model_ = MODEL_CAB[m];
        ir_rate_ = rate_;
    }

public:
    RedeyeDsp()
        : PluginLV2(), rate_(48000), conv_(), active_(false), model_(-1),
          inst_(0), model_ptr_(0), ir_model_(0xffffffffu), ir_rate_(0),
          ir_work_(0), ir_work_len_(0) {
        for (int i = 0; i < 8; ++i) params_[i] = 0;
        version = PLUGINLV2_VERSION;
        id = "redeye";
        name = N_("Redeye");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
    ~RedeyeDsp() {
        destroy_inst(inst_);
        free(ir_work_);
    }

    static void init_static(uint32_t rate, PluginLV2* p) {
        RedeyeDsp* s = static_cast<RedeyeDsp*>(p);
        s->rate_ = rate;
        if (s->inst_) {
            if (s->inst_->set_samplerate) s->inst_->set_samplerate(rate, s->inst_);
        } else {
            // eager-create the default model (0) at slot-assign time
            s->ensure_model(0);
        }
        s->ir_rate_ = 0; // force cab IR re-push at the new rate
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        RedeyeDsp* s = static_cast<RedeyeDsp*>(p);
        float* f = static_cast<float*>(data);
        // forward to the live instance too, so the host's post-move
        // re-connect (fx_move_slot) re-points the underlying DSP as well
        PluginLV2* inst = s->inst_;
        if (port < 8) {
            s->params_[port] = f;
            if (inst && inst->connect_ports) inst->connect_ports(port, f, inst);
        } else if (port == 10) {
            s->model_ptr_ = f;
        }
    }
    static int activate_static(bool start, PluginLV2* p) {
        RedeyeDsp* s = static_cast<RedeyeDsp*>(p);
        s->active_ = start;
        if (s->inst_ && s->inst_->activate_plugin)
            s->inst_->activate_plugin(start, s->inst_);
        if (start) s->conv_.reset();  // fresh streaming state on enable
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        RedeyeDsp* s = static_cast<RedeyeDsp*>(p);
        if (s->inst_ && s->inst_->clear_state) s->inst_->clear_state(s->inst_);
        s->conv_.reset();
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        RedeyeDsp* s = static_cast<RedeyeDsp*>(p);
        int m = 0;
        if (s->model_ptr_) { // integer param; round + clamp defensively
            m = (int)(*s->model_ptr_ + 0.5f);
            if (m < 0) m = 0;
            if (m > MODEL_COUNT - 1) m = MODEL_COUNT - 1;
        }
        if (m != s->model_ || !s->inst_) s->ensure_model(m);
        // model or rate change re-pushes the model's cab IR (pending-rebuild
        // policy: applied inside conv_.process below — bounded, same as
        // Cabinet's maybe_rebuild)
        if (s->ir_model_ != MODEL_CAB[m] || s->ir_rate_ != s->rate_)
            s->push_cab_ir(m);
        if (s->inst_ && s->inst_->mono_audio)
            s->inst_->mono_audio(count, input, output, s->inst_);
        else if (output != input)
            memcpy(output, input, (size_t)count * sizeof(float)); // fail-open
        s->conv_.process((size_t)count, output);
    }
    static void del_instance(PluginLV2* p) { delete static_cast<RedeyeDsp*>(p); }
};

const uint32_t RedeyeDsp::MODEL_CAB[RedeyeDsp::MODEL_COUNT] = { 17, 1, 1 };

PluginLV2* create() { return new RedeyeDsp(); }

} // namespace gxfx_redeye

// ---------------------------------------------------------------------------
// gxmetal_amp.lv2 / gxmetal_head.lv2 — Metal Amp / Metal Head
// (effects ids 78/79)
// ---------------------------------------------------------------------------
// Upstream gxmetal_{amp,head}.cpp are wrappers over the faust preamp
// classes (which live in gxfx_dsp_amps.cpp — they #include valve.h) + a
// FIXED cab_data_4x12 convolution stage (cab_table 0). The wrapper's
// `impf.compute(count, data, data, 10)` is an identity scaling (gain =
// 10² × 0.01 = 1.0, applied in place on the global table upstream — a
// quirk we do NOT replicate: the const table is handed to the convolver
// unmodified). The IR is pushed once per rate at first render
// (pending-rebuild policy). HIGHGAIN (port 6) is notOnGUI → never
// connected, class default holds. PortIndex = gxmetal_{amp,head}.h = ttl.

// gxmetal_{amp,head}.cc preamp factories live in gxfx_dsp_amps.cpp (the
// valve.h TU — see the metal comment block there).
PluginLV2* gxfx_ampsdsp_metalamp();
PluginLV2* gxfx_ampsdsp_metalhead();

namespace gxfx_metal {

class MetalDsp : public PluginLV2 {
private:
    uint32_t rate_;
    gxfx_conv::PartitionedConvolver conv_;
    PluginLV2* preamp_;
    bool have_ir_;      // IR pushed at this rate (pending until first render)
    // HIGHGAIN (ttl port 6) is notOnGUI → filtered from the param mirror →
    // the host never connects it. Upstream LV2 hosts still bind notOnGUI
    // ports to a default-valued buffer; the faust ctors leave the f*_*
    // pointers uninitialized, so we must do the same or compute() would
    // dereference a wild pointer. Held at the ttl default 0 (highgain off).
    float highgain_;

public:
    MetalDsp(PluginLV2* (*factory)(), const char* fx_id, const char* fx_name)
        : PluginLV2(), rate_(48000), conv_(), preamp_(factory()),
          have_ir_(false), highgain_(0.0f) {
        version = PLUGINLV2_VERSION;
        id = fx_id;
        name = fx_name;
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
    ~MetalDsp() {
        if (preamp_) {
            if (preamp_->activate_plugin) preamp_->activate_plugin(false, preamp_);
            if (preamp_->delete_instance) preamp_->delete_instance(preamp_);
        }
    }

    static void init_static(uint32_t rate, PluginLV2* p) {
        MetalDsp* s = static_cast<MetalDsp*>(p);
        s->rate_ = rate;
        if (s->preamp_ && s->preamp_->set_samplerate)
            s->preamp_->set_samplerate(rate, s->preamp_);
        if (s->preamp_ && s->preamp_->connect_ports)
            s->preamp_->connect_ports(6 /* HIGHGAIN */, &s->highgain_, s->preamp_);
        s->have_ir_ = false; // re-push the 4x12 IR at the new rate
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        MetalDsp* s = static_cast<MetalDsp*>(p);
        // TONE/DRIVE/PREGAIN/GAIN1 go straight to the preamp class (its
        // connect() ignores audio ports + HIGHGAIN by default-case)
        if (s->preamp_ && s->preamp_->connect_ports)
            s->preamp_->connect_ports(port, data, s->preamp_);
    }
    static int activate_static(bool start, PluginLV2* p) {
        MetalDsp* s = static_cast<MetalDsp*>(p);
        if (s->preamp_ && s->preamp_->activate_plugin)
            s->preamp_->activate_plugin(start, s->preamp_);
        if (start) s->conv_.reset();
        return 0;
    }
    static void clear_static(PluginLV2* p) {
        MetalDsp* s = static_cast<MetalDsp*>(p);
        if (s->preamp_ && s->preamp_->clear_state) s->preamp_->clear_state(s->preamp_);
        s->conv_.reset();
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        MetalDsp* s = static_cast<MetalDsp*>(p);
        if (!s->have_ir_) {
            // fixed 4x12 cab (cab_table 0), ×1.0 level — convolver resamples
            // from the table's own rate (cab.ir_sr) and rebuilds on THIS
            // render's process() call
            const CabDesc& cab = *getCabEntry(0).data;
            s->conv_.set_ir(cab.ir_data, (size_t)cab.ir_count,
                            (double)cab.ir_sr, (double)s->rate_);
            s->have_ir_ = true;
        }
        if (s->preamp_ && s->preamp_->mono_audio)
            s->preamp_->mono_audio(count, input, output, s->preamp_);
        else if (output != input)
            memcpy(output, input, (size_t)count * sizeof(float)); // fail-open
        s->conv_.process((size_t)count, output);
    }
    static void del_instance(PluginLV2* p) { delete static_cast<MetalDsp*>(p); }
};

PluginLV2* create_amp() {
    return new MetalDsp(gxfx_ampsdsp_metalamp, "metalamp", N_("Metal Amp"));
}
PluginLV2* create_head() {
    return new MetalDsp(gxfx_ampsdsp_metalhead, "metalhead", N_("Metal Head"));
}

} // namespace gxfx_metal

// namespace path: gxfx_cabinet::create(). Factory order MUST match the
// catalog row order (id 76 cabinet + 77 redeye, 78 metal amp, 79 metal
// head). Rows in gxfx_dsp_eleven.cpp stay NULL until the wave-flip task;
// the flip wires these four C++-linkage factories into GXFX_FACTORIES.
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_cabinet() { return gxfx_cabinet::create(); }
PluginLV2* gxfx_create_redeye() { return gxfx_redeye::create(); }
PluginLV2* gxfx_create_metalamp() { return gxfx_metal::create_amp(); }
PluginLV2* gxfx_create_metalhead() { return gxfx_metal::create_head(); }
