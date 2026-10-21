// gxfx_dsp_time.cpp — the delay/time family (gxfx catalog ids 45..48 and
// 53..56) as freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_time.cpp
// (the authoritative wrapper TU). Each guitarix class is pulled into its
// own namespace together with its PortIndex enum, transcribed VERBATIM
// from OctOBX — the enum values encode the LV2 ttl port indexes of the
// gx_<fx>.lv2 bundles (cross-checked against the gxfx_catalog.h rows for
// these ids) and a wrong enum silently routes params to the wrong
// controls. Factory order matches the manifest order (OctOBX
// src/gxfx-params.ts rows 45..56): duck_delay, duck_delay_st,
// digital_delay, digital_delay_st, [gxtape, gxtape_st, gxechocat,
// gxtubedelay — BLOCKED, see below], ts9, oc_2, classic_delay,
// classic_echo.
//
// Adapted from OctOBX, same rules as gxfx_dsp_distortion.cpp /
// gxfx_dsp_drive.cpp:
//  - all guitarix sources are included via RELATIVE paths from this
//    dir into ../../../vendor/guitarix/trunk/... so build.rs needs no
//    new include dirs;
//  - beat.h (B2N note-value→ms helper, included by both digital_delay
//    .cc files) is staged at GLOBAL scope first — the include-once
//    trick — so both namespaces see ::B2N; it is `static inline`, so
//    per-TU linkage means no cross-TU ODR exposure.
//
// BLOCKED ids 49..52 (gxtape, gxtape_st, gxechocat, gxtubedelay): each
// of those faust .cc files #includes bare-named circuit-table headers
// (input12au7*, output12au7*, the seven copicat*, input/output12ax7*)
// that exist ONLY in vendor/guitarix/trunk/src/LV2/DSP/circuit_tables/
// — a directory on NO include path of build.rs (verified empirically:
// gxtape.cc fails with 'input12au7_neg_table.h' file not found under
// build.rs's exact flag set). A #include directive naming an unfindable
// file is a hard preprocessor error, so no in-TU staging (the trick
// gxfx_dsp_drive.cpp uses for trany.h) can reach them — there the
// wrapper header itself was findable in DSP/ and only its nested table
// includes needed redirecting; here the table headers themselves are
// the unfindable files. Unlock (either, in a follow-up task with the
// rights to do it): add `-I vendor/guitarix/trunk/src/LV2/DSP/
// circuit_tables` to build.rs's include set (exactly what OctOBX's
// documented compile line for this TU passes), or stage copies of the
// 16 table headers in this dir (which IS on the include path) the way
// the amps WIP does for the alembic/w20 tables. Until then this TU
// ships the other eight effects and the four stay null factories.
//
// MEMORY (memory-class, catalog memory=1 for 45..48, 55, 56): the four
// bundle classes carry a `double fVec0/fVec2[524288]` class member —
// 4 MB per object at CREATION, not lazy (mono effects cost 8 MB per
// slot dual-mono, the _st ones 4 MB as one stereo instance); the
// classic delay/echo orphans lazily `new float[524288]` (2 MB) in
// activate(true). The engine already caps concurrently created
// memory-class instances (gxfx_catalog.h flags them); DSP create/
// activate stays eager at message time, never inside process().
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete
// overloads and the memset definition for the whole gxfx archive (the
// classes' new/delete expressions here, including Ts9Dsp's, resolve
// against them at archive link time); this TU defines NONE of them, no
// extern "C", no emscripten.
//
// Statelessness: all param storage and strip state live on the Rust
// side; this is only the DSP classes + factories. These ids stay
// unavailable (null factories in gxfx_dsp_eleven.cpp's table) until a
// follow-up task flips those rows — this TU enters the archive
// unlinked until then.

#include "gxfx_prelude.h"
#include <math.h> // C-scope libm: ts9nonlin.h (gxts9) calls bare copysign()
                  // and includes no math header itself — it rides the
                  // includer's declarations. Same as gxfx_dsp_drive.cpp for
                  // clipping.h: the prelude's <cmath> is covered natively,
                  // but the wasm shim declares copysign only in <math.h>.
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/gx_pluginlv2.h"

// Include-once trick: both digital_delay .cc files #include "beat.h"
// inside what would be their wrapper namespace — pulled in at GLOBAL
// scope first, the guard hides it from both namespaces; the classes
// reach the static ::B2N via unqualified lookup.
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/beat.h"

// --- gx_duck_delay.lv2 (bundle-local duck_delay.cc) -------------------------
// Ducking delay (dsp2cc class; PortIndex from gx_duck_delay.h — the
// .cc's own trailing enum comment is the stale dsp order).
namespace gxfx_duck_delay {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    AMOUNT, ATTACK, FEEDBACK, RELESE, TIME,
    BYPASS,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_duck_delay.lv2/duck_delay.cc"
}

// --- gx_duck_delay_st.lv2 (bundle-local duck_delay_st.cc) -------------------
// Stereo ducking delay with coloration / ping-pong (PortIndex from
// gx_duck_delay_st.h = ttl indexes 0..12).
namespace gxfx_duck_delay_st {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT1,
    AMOUNT, ATTACK, COLORATION, EFFECT, FEEDBACK, PINGPONG, RELEASE, TIME,
    BYPASS,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_duck_delay_st.lv2/duck_delay_st.cc"
}

// --- gx_digital_delay.lv2 (bundle-local digital_delay.cc) -------------------
// BPM-synced delay (plain/presence/tape/tape2 modes). PortIndex from
// gx_digital_delay.h — DD_CONTROL (atom in), DD_NOTIFY (bpm meter out),
// SYNC + HOSTBPM (wrapper-level LV2 host-tempo ports the class reads
// every compute but the generator skips as params — see the factory
// below) and BYPASS are part of the wrapper space but never connected
// by the host. fVec2[524288] doubles = the 4 MB-at-creation member.
namespace gxfx_digital_delay {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    BPM, FEEDBACK, GAIN, HIGHPASS, HOWPASS, LEVEL, MODE, NOTES,
    DD_CONTROL, DD_NOTIFY, SYNC, HOSTBPM, BYPASS,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_digital_delay.lv2/digital_delay.cc"
}

// --- gx_digital_delay_st.lv2 (bundle-local digital_delay_st.cc) -------------
// Stereo twin (PortIndex from gx_digital_delay_st.h). Unlike the mono
// class it has NO sync logic (no fcheckbox member at all) — the factory
// needs no scratch parking.
namespace gxfx_digital_delay_st {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT1,
    BPM, FEEDBACK, GAIN, HIGHPASS, HOWPASS, LEVEL, MODE, NOTES,
    DD_CONTROL, DD_NOTIFY, SYNC, HOSTBPM, BYPASS,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_digital_delay_st.lv2/digital_delay_st.cc"
}

// --- gxts9.lv2 (bundle-local ts9sim.cc + ts9nonlin.h/.cc) -------------------
// Ibanez TS-9 Tubescreamer circuit sim. The bundle ships a plain
// circuit-sim class (ts9sim — NOT a PluginLV2); upstream's gxts9.cpp
// wrapper owns the LV2 plumbing. Ts9Dsp below is the headless
// equivalent of that wrapper (ts9sim by value; activate ->
// clear_state, and set_samplerate -> init also clears, like upstream).
// ts9sim.cc #defines bare max/min macros (contained upstream by
// inclusion order) — #undef right after the include so the prelude's
// using-declarations keep serving later classes. Menu category is
// DRIVE (Tubescreamer-style overdrive), not delay — the TU placement
// follows the manifest, the category follows the ear.
namespace gxfx_ts9 {
typedef enum {
    TS9_LEVEL, TS9_TONE, TS9_DRIVE,
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    BYPASS,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gxts9.lv2/ts9sim.cc"
#undef max
#undef min

// Headless stand-in for upstream's gxts9.cpp LV2 wrapper: adapts the
// plain ts9sim circuit class to PluginLV2 (mono; the host runs it
// dual-mono). The empty ts9sim ctor leaves its rec/vec states
// uninitialized, but init() at set_samplerate and activate(true) both
// clear_state before the first compute, so no recycled-heap garbage
// can reach the audio path.
class Ts9Dsp : public PluginLV2 {
private:
    ts9sim sim_;
    static Ts9Dsp* self(PluginLV2* p) { return static_cast<Ts9Dsp*>(p); }
    static void init_static(uint32_t rate, PluginLV2* p) {
        ts9sim::init_static(rate, &self(p)->sim_);
    }
    static void compute_static(int count, FAUSTFLOAT* in, FAUSTFLOAT* out, PluginLV2* p) {
        ts9sim::run_static((uint32_t)count, in, out, &self(p)->sim_);
    }
    static int activate_static(bool start, PluginLV2* p) {
        if (start) ts9sim::clear_state_static(&self(p)->sim_);
        return 0;
    }
    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        ts9sim::connect_static(port, data, &self(p)->sim_);
    }
    static void clear_static(PluginLV2* p) {
        ts9sim::clear_state_static(&self(p)->sim_);
    }
    static void del_instance(PluginLV2* p) { delete self(p); }

public:
    Ts9Dsp() : PluginLV2(), sim_() {
        version = PLUGINLV2_VERSION;
        id = "ts9";
        name = N_("TS-9");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        clear_state = clear_static;
        delete_instance = del_instance;
    }
};
} // namespace gxfx_ts9

// --- gx_oc_2.lv2 (bundle-local oc_2.cc + triggers_logic.h) ------------------
// Boss OC-2 style octave divider (two squared-octave voices + direct).
// oc_2.cc #includes "triggers_logic.h" relative to its own dir. Menu
// category SPECIAL — the frozen vocabulary has no pitch group.
namespace gxfx_oc_2 {
typedef enum {
    EFFECTS_OUTPUT, EFFECTS_INPUT,
    DIRECT, OCTAVE1, OCTAVE2,
} PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/gx_oc_2.lv2/oc_2.cc"
}

// --- classic faust orphans ---------------------------------------------------
// Mono delay.cc / echo.cc — named "Classic Delay"/"Classic Echo" in the
// menu; id 8 "Delay" / id 9 "Echo" (eleven TU) are the STEREO
// stereodelay.cc / stereoecho.cc classes. PortIndex = each .cc's own
// trailing enum comment (the orphan generator's source). The 2 MB
// delay line is new'd in activate(true), freed in activate(false).

namespace gxfx_classic_delay {
typedef enum { DELAY, GAIN } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/delay.cc"
}

namespace gxfx_classic_echo {
typedef enum { PERCENT, TIME } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/echo.cc"
}

// namespace paths: gxfx_duck_delay::duck_delay::plugin() etc. Factory
// order MUST match the generator manifest order (ids 45..48, 53..56;
// 49..52 ride the circuit_tables blocker noted in the header comment).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_duck_delay() { return gxfx_duck_delay::duck_delay::plugin(); }
PluginLV2* gxfx_create_duck_delay_st() { return gxfx_duck_delay_st::duck_delay_st::plugin(); }

// digital_delay (mono) reads its SYNC checkbox and HOSTBPM slider EVERY
// compute() (`if (int(fcheckbox0)) fslider6 = fslider8;`) but the ctor
// leaves those pointer members uninitialized — and since SYNC/HOSTBPM
// are wrapper-level host-tempo ports the generator skips as params,
// they would stay unconnected and dereference recycled-heap garbage
// (the vibe.cc failure class). Park them on TU-local scratch with the
// upstream-default values (sync OFF, 120 BPM). (The STEREO class has
// no sync logic — only fslider0..7 — and needs nothing.)
static float g_dd_scratch_sync = 0.0f;     // SYNC off: the BPM param rules
static float g_dd_scratch_hostbpm = 120.0f;
PluginLV2* gxfx_create_digital_delay() {
    PluginLV2* p = gxfx_digital_delay::digital_delay::plugin();
    if (p && p->connect_ports) {
        p->connect_ports((uint32_t)gxfx_digital_delay::SYNC, &g_dd_scratch_sync, p);
        p->connect_ports((uint32_t)gxfx_digital_delay::HOSTBPM, &g_dd_scratch_hostbpm, p);
    }
    return p;
}

PluginLV2* gxfx_create_digital_delay_st() { return gxfx_digital_delay_st::digital_delay_st::plugin(); }
PluginLV2* gxfx_create_ts9() { return new gxfx_ts9::Ts9Dsp(); }
PluginLV2* gxfx_create_oc_2() { return gxfx_oc_2::oc_2::plugin(); }
PluginLV2* gxfx_create_classic_delay() { return gxfx_classic_delay::delay::plugin(); }
PluginLV2* gxfx_create_classic_echo() { return gxfx_classic_echo::echo::plugin(); }
