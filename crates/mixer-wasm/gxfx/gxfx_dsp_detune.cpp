// gxfx_dsp_detune.cpp — "Detune" (catalog id 80, category special) as
// freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm): gx_detune.lv2's bundle-LOCAL smbPitchShift phase
// vocoder (Stephan M. Bernsee, http://www.dspdimension.com, adapted for
// guitarix by Hermann Meyer 2014; the Wide Open License), unmodified.
//
// Ported from OctOBX wasm/obxd/gxfx_dsp_detune.cpp (the authoritative
// wrapper TU); namespaces, PortIndex enum and factory are VERBATIM.
// Adaptations for this repo, each mechanical:
//   * include paths (vendor submodule depth, gx_pluginlv2.h/gx_resampler.h
//     via -I vendor/guitarix/trunk/src/LV2/DSP);
//   * <fftw3.h> resolves to gxfx/shim/fftw3.h (the kissfft-backed fftw3
//     API shim — header-only, complex-DFT subset) on BOTH targets: the
//     freestanding wasm build searches gxfx/shim first, native builds go
//     through the gxfx/fftw3.h forwarder;
//   * -fno-exceptions: mem_alloc's `try { ... } catch(...) { return; }`
//     is neutralized below via `try`/`catch` macros (looper TU
//     precedent). This runtime's global operator new (gxfx_dsp_eleven.cpp)
//     routes to malloc and returns null instead of throwing, so the
//     handler was already unreachable — the transform only removes the
//     syntax.
//
// Upstream wiring (gx_detune.cpp, the LV2 wrapper — bypassed entirely like
// every gxfx port): instantiate → set_buffersize(host bufsize) +
// set_samplerate(rate) → connect ports → activate(true) (mem_alloc: FFT
// plans + tables) → run() per block. run_dsp_() then schedules the LV2
// WORKER whenever the LATENCY port value changed (or bufsize != n); the
// worker (do_work_mono) ONLY does set_buffersize + change_latency_static
// — a mem_free/mem_alloc plan rebuild. No analysis thread, no locks.
//
// Inline decision (engine renders exactly 128-sample quanta): the
// bufsize-vs-n trigger can never fire; the LATENCY trigger is inlined in
// compute_static BELOW the delegated block — the class refreshes its
// `latency` member at the top of every PROCESSED block, and upstream's
// worker ran after run() returned, so post-block ordering sees the same
// fresh value. The rebuild is bounded (two kissfft plans + ~6k floats —
// the same budget class as Cabinet's pending IR rebuild).
//
// First-activation quirk (faithful + hardened): upstream's first
// mem_alloc runs BEFORE any block has refreshed the `latency` member, so
// its frame-size switch reads uninitialized memory (default branch ==
// case 0 at bufsize 128: fftFrameSize 512). The wrapper's last_latency_
// sentinel (-1, the param min is 0) forces exactly ONE deterministic
// change_latency rebuild after the first processed block, after which the
// plans are provably built from the connected param value.
//
// Memory footprint: each smbPitchShift object embeds fixed
// MAX_FRAME_LENGTH(8096)-sized frame arrays (~420 KB) — ~840 KB per slot
// dual-mono (sub-MB: the generated catalog row 80 carries memory=0, so
// the 96-instance memory-class cap does not apply).
//
// Param surface = gx_detune.ttl control inputs DETUNE/OCTAVE/COMPENSATE/
// LATENCY/WET/DRY/LOW/MIDDLELOW/MIDDLETREBLE/TREBLE (ttl ports 2..11);
// BYPASS (lv2:enabled designation) is generator-filtered. The latency
// OUTPUT port (LATENCYREPORT, ttl port 12) is parked on wrapper-local
// scratch: the class dereferences latencyr_ after every processed block
// and the host never connects output ports (graphiceq meter precedent).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (detune's
// new/delete'd plans + tables resolve against them at archive link
// time); this TU defines NONE of them, no extern "C", no emscripten.
// NOT yet wired into gxfx_dsp_eleven.cpp's factory table (row 80 stays
// null until the final wave's honesty-test flip; until then this object
// rides the archive unreferenced).
//
// Standalone syntax check (native):
//   clang++ -std=c++20 -O2 -fsyntax-only -Wno-vla-cxx-extension \
//     -I crates/mixer-wasm/gxfx \
//     -I vendor/guitarix/trunk/src/LV2/DSP \
//     -I vendor/guitarix/trunk/src/zita-resampler-1.1.0 \
//     crates/mixer-wasm/gxfx/gxfx_dsp_detune.cpp
#include "gxfx_prelude.h"

#include <fftw3.h> // gxfx/shim/fftw3.h (via shim dir / gxfx/fftw3.h forwarder) — BEFORE detune.cc

#include <assert.h>

#include "gx_pluginlv2.h"  // via -I vendor/guitarix/trunk/src/LV2/DSP
#include "gx_resampler.h"  // via -I vendor/guitarix/trunk/src/LV2/DSP

// PortIndex = gx_detune.h (the wrapper enum detune.cc's connect() switches
// on). Re-declared here instead of including the header: gx_detune.h pulls
// the LV2 extension headers (<lv2.h>, worker, urid, ...) which do not exist
// in the headless build (redeye precedent in gxfx_dsp_conv.cpp).
namespace gxfx_detune_dsp {
typedef enum {
    EFFECTS_OUTPUT = 0, EFFECTS_INPUT = 1,
    DETUNE = 2, OCTAVE = 3, COMPENSATE = 4, LATENCY = 5,
    WET = 6, DRY = 7, LOW = 8, MIDDLELOW = 9, MIDDLETREBLE = 10,
    TREBLE = 11, LATENCYREPORT = 12, BYPASS = 13,
} PortIndex;

// The bundle-local DSP class — unmodified, namespaced like every gxfx DSP
// include. detune.cc opens its own `namespace detune {`, so the class lands
// at gxfx_detune_dsp::detune::smbPitchShift.
//
// -fno-exceptions neutralization (scoped to the vendor include): under
// this runtime operator new returns null instead of throwing, so
// mem_alloc's catch arm is unreachable — compile it out, keep the flow.
#define try if (true)
#define catch(...) if (false)
#include "../../../vendor/guitarix/trunk/src/LV2/gx_detune.lv2/detune.cc"
#undef catch
#undef try
} // namespace gxfx_detune_dsp

namespace gxfx_detune {

// Host render quantum — the only block size the mixer engine renders.
enum { BUFSIZE = 128 };

class DetuneDsp : public PluginLV2 {
private:
    PluginLV2* d_;
    float* latency_ptr_; // connected LATENCY param mirror slot (port 5)
    float latency_out_;  // LATENCYREPORT scratch (port 12) — see header
    float last_latency_; // LATENCY value the plans were built for; -1
                         // sentinel forces one deterministic rebuild after
                         // the first processed block

public:
    DetuneDsp()
        : PluginLV2(), d_(gxfx_detune_dsp::detune::plugin()),
          latency_ptr_(0), latency_out_(0.0f), last_latency_(-1.0f) {
        version = PLUGINLV2_VERSION;
        id = "detune";
        name = N_("Detune");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        delete_instance = del_instance;
    }
    ~DetuneDsp() {
        if (d_) {
            if (d_->activate_plugin) d_->activate_plugin(false, d_);
            if (d_->delete_instance) d_->delete_instance(d_);
        }
    }

    // upstream Gx_detune_::init_dsp_(): buffersize THEN samplerate, plus
    // the LATENCYREPORT scratch parking.
    static void init_static(uint32_t rate, PluginLV2* p) {
        DetuneDsp* s = static_cast<DetuneDsp*>(p);
        if (!s->d_) return;
        gxfx_detune_dsp::detune::smbPitchShift::set_buffersize(s->d_, BUFSIZE);
        if (s->d_->set_samplerate) s->d_->set_samplerate(rate, s->d_);
        if (s->d_->connect_ports)
            s->d_->connect_ports(gxfx_detune_dsp::LATENCYREPORT,
                                 &s->latency_out_, s->d_);
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        DetuneDsp* s = static_cast<DetuneDsp*>(p);
        if (port == gxfx_detune_dsp::LATENCY)
            s->latency_ptr_ = static_cast<float*>(data);
        if (s->d_ && s->d_->connect_ports) s->d_->connect_ports(port, data, s->d_);
    }
    static int activate_static(bool start, PluginLV2* p) {
        DetuneDsp* s = static_cast<DetuneDsp*>(p);
        if (s->d_ && s->d_->activate_plugin) s->d_->activate_plugin(start, s->d_);
        return 0;
    }
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        DetuneDsp* s = static_cast<DetuneDsp*>(p);
        if (s->d_ && s->d_->mono_audio) {
            s->d_->mono_audio(count, input, output, s->d_);
        } else if (output != input) {
            memcpy(output, input, (size_t)count * sizeof(float)); // fail-open
        }
        // Upstream's LV2 worker (do_work_mono), INLINED: rebuild the FFT
        // plans when the LATENCY param value changed. Runs AFTER the
        // delegated block — the class's `latency` member is refreshed at
        // the top of every processed block, and upstream's worker ran
        // after run() returned, so change_latency() here sees the same
        // fresh value (change_latency is a no-op unless mem_allocated).
        if (s->d_ && s->latency_ptr_) {
            const float v = *s->latency_ptr_;
            if (v != s->last_latency_) {
                s->last_latency_ = v;
                gxfx_detune_dsp::detune::smbPitchShift::change_latency_static(s->d_);
            }
        }
    }
    static void del_instance(PluginLV2* p) { delete static_cast<DetuneDsp*>(p); }
};

PluginLV2* create() { return new DetuneDsp(); }

} // namespace gxfx_detune

// namespace path: gxfx_detune::create(). Factory order MUST match the
// generator manifest order (id 80 detune, after the conv family).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_detune() { return gxfx_detune::create(); }
