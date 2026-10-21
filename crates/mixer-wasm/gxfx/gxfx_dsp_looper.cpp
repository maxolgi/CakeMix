// gxfx_dsp_looper.cpp — "Live Looper" (catalog id 82, category special)
// as freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm): gx_livelooper.lv2's bundle-LOCAL, hand-tweaked
// faust class (livelooper.cc — NOT the gx_head/engine LiveLooper),
// namespace-wrapped like every gxfx DSP include. The class is pure
// audio + control ports: no mutexes, no worker thread — only its wave
// save/load paths touch libsndfile, which the gxfx/shim/sndfile.hh
// include-order stub turns into graceful no-ops (sf_open always fails
// → load_array returns 0, save_to_wave skips; loops start EMPTY each
// session — see the shim header). getenv("HOME") also runs inside
// those paths: natively the real libc serves it; on the freestanding
// wasm target the Rust c_floor defines it to return NULL and the
// <string> shim's null-tolerant ctor degrades the loop dir to "".
//
// Ported from OctOBX wasm/obxd/gxfx_dsp_looper.cpp (the authoritative
// wrapper TU); namespaces, PortIndex enum and factory are VERBATIM.
// Adaptations for this repo, each mechanical:
//   * include paths (vendor submodule depth, gx_pluginlv2.h via -I);
//   * the sndfile stub is included as "shim/sndfile.hh" — native
//     builds do not put shim/ on the include path and must never pick
//     up a real system sndfile.hh (undefined sf_* at link); the string
//     shim is included the same way ("shim/string", BOTH targets):
//     the real libstdc++ string's members reference
//     std::__throw_length_error/__throw_logic_error, which Rust's
//     nodefaultlibs native link (no libstdc++) cannot resolve;
//   * -fno-exceptions: mem_alloc's `try { new ... } catch(...) {}`
//     is neutralized below via `try`/`catch` macros. This runtime's
//     global operator new (gxfx_dsp_eleven.cpp) routes to malloc and
//     returns null instead of throwing, so the handler was already
//     unreachable — the transform only removes the syntax.
//
// Port space: transcribed from the bundle's gx_livelooper.h wrapper
// header (= the ttl indexes 0..48: out/in audio, then clip1..synct);
// livelooper.cc's own trailing PortIndex comment is the same order minus
// the audio pair. 39 control inputs (params) + 8 control OUTPUTS:
// bar1..4 (remaining record-time meters, seconds — written every compute
// from rectime*) and playh1..4 (play-head position, 0..1000 per-mille —
// held at 0 while the matching rec flag is on). The rack in src/gxfx.rs
// connects them into the slot's out-port row (bar1..4 → indices 0..3,
// playh1..4 → 4..7; mono slot → dual-mono pair, both halves write the
// same row — tuner precedent, harmless), readable via fx_out_param.
//
// MEMORY (OctOBX fx2plan decision #5, kept as-is): TAPESIZE 4194304
// floats = 16 MiB per tape, 4 tapes per LiveLooper = 64 MiB per DSP
// instance, `new`'d LAZILY in activate(true) (mem_alloc) — not at
// set_slot's eager create. The looper is MONO (one ttl audio input →
// host runs it dual-mono), so an ENABLED looper slot costs 2 x 64 =
// 128 MiB, landing at fx_set_enabled (message) time on the first
// render after enable (plus one clear_state_f() pass that zeroes all
// tapes again). The deactivation path (slot clear / set_slot /
// restore) runs activate(false) → save_array (stub no-op) → mem_free.
// One looper ≈ 128 MiB does NOT fit under the generic 96-instance
// memory-class cap (that budget counts ~4 MiB tapes); the rack keeps
// a SEPARATE looper cap (LOOPER_CAP = 4 created looper slots ≈ 512 MiB
// worst case), enforced at set_slot.
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete
// overloads and the memset definition for the whole gxfx archive (the
// looper's new/delete[] tape allocations resolve against them at
// archive link time); this TU defines NONE of them, no extern "C", no
// emscripten. NOT yet wired into gxfx_dsp_eleven.cpp's factory table
// (row 82 stays null until the final wave's honesty-test flip; until
// then this object rides the archive unreferenced).
//
// Standalone syntax check (native):
//   clang++ -std=c++20 -O2 -fsyntax-only -Wno-vla-cxx-extension \
//     -I crates/mixer-wasm/gxfx \
//     -I vendor/guitarix/trunk/src/LV2/DSP \
//     crates/mixer-wasm/gxfx/gxfx_dsp_looper.cpp
#include "gxfx_prelude.h"

// libsndfile stub FIRST and at GLOBAL scope (the wrapper's <sndfile.hh>
// slot): livelooper.cc's load_from_wave/save_to_wave reference the C API
// unqualified. <sys/stat.h> (stat/mkdir in save_array) and <cstdio>
// (mem_alloc's OOM fprintf) cover the remaining wrapper-side includes —
// on the freestanding wasm target all three resolve to gxfx/shim/
// (stat/mkdir/getenv fail closed from the Rust c_floor, matching the
// sndfile stub's always-fail contract); natively the system headers
// serve unchanged — EXCEPT <string>, pulled as "shim/string" on BOTH
// targets: the real libstdc++ string's members reference
// std::__throw_length_error/__throw_logic_error, which Rust's
// nodefaultlibs native link (no libstdc++) cannot resolve, and the
// shim is null-tolerant where libstdc++ would be UB (getenv → NULL).
#include "shim/sndfile.hh"
#include <sys/stat.h>
#include <cstdio>
#include "shim/string"

#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP

// --- gx_livelooper.lv2 (bundle-local livelooper.cc) ------------------------
// LiveLooper ships PluginLV2-shaped STATIC methods keyed on LiveLooper*
// (init_static/activate_static/compute_static/connect_static/
// del_instance + plugin()), but is not itself a PluginLV2 — the wrapper's
// Gx_livelooper_ provided that vtable upstream. LooperDsp below is our
// headless equivalent (TunerDsp pattern).
namespace gxfx_livelooper {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    clip1, clip2, clip3, clip4,
    clips1, clips2, clips3, clips4,
    speed1, speed2, speed3, speed4,
    bar1, bar2, bar3, bar4,
    gain, level1, level2, level3, level4, mix,
    play1, play2, play3, play4,
    rplay1, rplay2, rplay3, rplay4,
    playh1, playh2, playh3, playh4,
    rec1, rec2, rec3, rec4,
    reset1, reset2, reset3, reset4,
    rback1, rback2, rback3, rback4,
    synct,
} PortIndex;

// -fno-exceptions neutralization (scoped to the vendor include): under
// this runtime operator new returns null instead of throwing, so
// mem_alloc's catch arm is unreachable — compile it out, keep the flow.
#define try if (true)
#define catch(...) if (false)
#include "../../../vendor/guitarix/trunk/src/LV2/gx_livelooper.lv2/livelooper.cc"
#undef catch
#undef try

} // namespace gxfx_livelooper

namespace gxfx_looper {

class LooperDsp : public PluginLV2 {
private:
    gxfx_livelooper::livelooper::LiveLooper* lo_;

public:
    LooperDsp()
        : PluginLV2(),
          lo_(gxfx_livelooper::livelooper::plugin()) {
        version = PLUGINLV2_VERSION;
        id = "livelooper";
        name = N_("Live Looper");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        delete_instance = del_instance;
    }

    static void init_static(uint32_t rate, PluginLV2* p) {
        LooperDsp* s = static_cast<LooperDsp*>(p);
        if (s->lo_) gxfx_livelooper::livelooper::LiveLooper::init_static(rate, s->lo_);
    }

    // activate(true) lazily news the 4 x 16 MiB tapes + zeroes them
    // (mem_alloc + clear_state_f); activate(false) saves (stub no-op) and
    // frees. Called by the rack's set_enabled / slot teardown.
    static int activate_static(bool start, PluginLV2* p) {
        LooperDsp* s = static_cast<LooperDsp*>(p);
        if (!s->lo_) return 0;
        return gxfx_livelooper::livelooper::LiveLooper::activate_static(start, s->lo_);
    }

    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        LooperDsp* s = static_cast<LooperDsp*>(p);
        if (s->lo_) gxfx_livelooper::livelooper::LiveLooper::connect_static(port, data, s->lo_);
    }

    // In-place safe: compute() reads input0[i] before writing output0[i].
    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        LooperDsp* s = static_cast<LooperDsp*>(p);
        if (!s->lo_) {
            if (input != output) memcpy(output, input, (size_t)count * sizeof(float));
            return;
        }
        gxfx_livelooper::livelooper::LiveLooper::compute_static(count, input, output, s->lo_);
    }

    static void del_instance(PluginLV2* p) {
        LooperDsp* s = static_cast<LooperDsp*>(p);
        if (s->lo_) gxfx_livelooper::livelooper::LiveLooper::del_instance(s->lo_); // dtor → activate(false) → mem_free
        delete s;
    }
};

PluginLV2* create() { return new LooperDsp(); }

} // namespace gxfx_looper

// namespace path: gxfx_looper::create(). Factory order MUST match the
// generator manifest order (id 82 livelooper, after the tuner).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_livelooper() { return gxfx_looper::create(); }
