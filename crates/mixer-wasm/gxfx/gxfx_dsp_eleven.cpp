// gxfx_dsp_eleven.cpp — the v1 "eleven" guitarix faust DSP effects as
// freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm), plus the archive-wide factory table wiring every
// ported family's rows (ids 0..75 live; 76..82 await their TUs).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp.cpp (the
// authoritative wrapper TU). Each faust class is pulled into its own
// namespace together with its PortIndex enum, transcribed VERBATIM from
// OctOBX — the enum values encode the LV2 ttl port indexes of the
// gx_<fx>.lv2 bundles and are proven in production; a wrong enum silently
// routes params to the wrong controls. The DSP sources are guitarix
// (GPLv2+) faust-generated classes consumed straight from the pinned
// vendor/guitarix submodule at trunk/src/LV2/faust-generated/. Build
// wiring (build.rs compiling this TU natively and for
// wasm32-unknown-unknown over gxfx/shim/) lands in a separate task.
//
// bossds1.cc/bmp.cc are intentionally NOT here: they depend on
// gx_resample/zita (resampler) and distortion + bigmuffpi ride their own
// TU, gxfx_dsp_distortion.cpp. THIS is the only TU that may define the
// global operator new/delete overloads and memset (there is no C++
// runtime on the freestanding target — they route to C malloc/free);
// gxfx_dsp_distortion.cpp must not.
//
// This TU is a stateless factory + per-instance dispatcher: ALL param
// storage, chaining, pair logic and strip state live on the Rust side.
// Each instance handle owns exactly one PluginLV2 whose control ports
// point into caller-owned (Rust) float storage via gxfx_connect_port.

#include "gxfx_prelude.h"
#include "gx_pluginlv2.h" // via -I vendor/guitarix/trunk/src/LV2/DSP
#include "gxfx_catalog.h"

// Global new/delete → C malloc/free for the whole TU chain (the faust
// classes' plugin()/del_instance() and their lazily-allocated delay
// buffers ride these). The sized-deallocation overloads are required
// alongside the unsized ones: since C++14, `delete` of a complete type
// prefers the (implicitly declared, library-provided) sized form — without
// our own definition those references would leave an undefined _ZdlPvm
// in the object. Together with memset below, this keeps the object's
// undefined surface to exactly the malloc/free family + libm.
void* operator new(size_t sz) { return malloc(sz); }
void* operator new[](size_t sz) { return malloc(sz); }
void operator delete(void* p) noexcept { free(p); }
void operator delete[](void* p) noexcept { free(p); }
void operator delete(void* p, size_t) noexcept { free(p); }
void operator delete[](void* p, size_t) noexcept { free(p); }

// memset: clang -O2 lowers the faust clear_state_f zeroing loops (vendor
// code, unpatchable) into memset libcalls natively. Defining it here keeps
// the object freestanding-clean; on wasm the loops inline and this stays
// unreferenced (linker-dropped). As with the new/delete overloads, this is
// the only gxfx TU that may define it.
extern "C" void* memset(void* dst, int value, size_t n)
{
    unsigned char* d = (unsigned char*)dst;
    unsigned char v = (unsigned char)value;
    while (n--) {
        *d++ = v;
    }
    return dst;
}

namespace gxfx_wah {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, VOLUME, HOTPOTZ } PortIndex;
#include "gcb_95.cc"
}

namespace gxfx_overdrive {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, SCREAM } PortIndex;
#include "scream.cc"
}

namespace gxfx_compressor {
typedef enum { RATIO, KNEE, THRESHOLD, RELEASE, ATTACK, EFFECTS_OUTPUT, EFFECTS_INPUT, BYPASS } PortIndex;
#include "compressor.cc"
}

namespace gxfx_chorus {
typedef enum { LEVEL, DELAY, DEPTH, FREQ, EFFECTS_OUTPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT, EFFECTS_INPUT1, BYPASS } PortIndex;
#include "chorus.cc"
}

namespace gxfx_flanger {
typedef enum { DEPTH, WIDTH, FREQ, FEEDBACK, WET, MIX, EFFECTS_OUTPUT, EFFECTS_INPUT } PortIndex;
#include "gx_flanger.cc"
}

namespace gxfx_phaser {
typedef enum { WET_DRY, LEVEL, SPEED, EFFECTS_OUTPUT, EFFECTS_INPUT } PortIndex;
#include "phaser_mono.cc"
}

namespace gxfx_tremolo {
typedef enum { WET_DRY, SINE, DEPTH, FREQ, EFFECTS_OUTPUT, EFFECTS_INPUT, RESET } PortIndex;
#include "tremolo.cc"
}

namespace gxfx_delay {
typedef enum { INVERT, R_GAIN, R_DELAY, L_GAIN, L_DELAY, LFOFREQ, LINK, EFFECTS_OUTPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT, EFFECTS_INPUT1, BYPASS } PortIndex;
#include "stereodelay.cc"
}

namespace gxfx_echo {
typedef enum { INVERT, PERCENT_R, TIME_R, PERCENT_L, TIME_L, LFOFREQ, LINK, EFFECTS_OUTPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT, EFFECTS_INPUT1, BYPASS } PortIndex;
#include "stereoecho.cc"
}

namespace gxfx_reverb {
typedef enum { WET_DRY, LFOFREQ, ROOMSIZE, DAMP, INVERT, EFFECTS_OUTPUT, EFFECTS_OUTPUT1, EFFECTS_INPUT, EFFECTS_INPUT1 } PortIndex;
#include "stereoverb.cc"
}

// Factory functions (OctOBX naming; C++ linkage — the extern "C" surface
// below is the Rust-side ABI).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_wah() { return gxfx_wah::gcb_95::plugin(); }
PluginLV2* gxfx_create_overdrive() { return gxfx_overdrive::scream::plugin(); }
PluginLV2* gxfx_create_compressor() { return gxfx_compressor::compressor::plugin(); }
PluginLV2* gxfx_create_chorus() { return gxfx_chorus::chorus::plugin(); }
PluginLV2* gxfx_create_flanger() { return gxfx_flanger::gx_flanger::plugin(); }
PluginLV2* gxfx_create_phaser() { return gxfx_phaser::phaser_mono::plugin(); }
PluginLV2* gxfx_create_tremolo() { return gxfx_tremolo::tremolo::plugin(); }
PluginLV2* gxfx_create_delay() { return gxfx_delay::stereodelay::plugin(); }
PluginLV2* gxfx_create_echo() { return gxfx_echo::stereoecho::plugin(); }
PluginLV2* gxfx_create_reverb() { return gxfx_reverb::stereoverb::plugin(); }

// Distortion (catalog id 2) rides gxfx_dsp_distortion.cpp (same archive,
// separate TU: it alone pulls gx_resample/zita link-complete — which is
// also why bigmuffpi, id 71, lives there).
PluginLV2* gxfx_create_distortion();
PluginLV2* gxfx_create_bigmuffpi();

// Drive + dynamics family (ids 11..27): gxfx_dsp_drive.cpp.
PluginLV2* gxfx_create_fuzzface();
PluginLV2* gxfx_create_fuzzfacefm();
PluginLV2* gxfx_create_fumaster();
PluginLV2* gxfx_create_hornet();
PluginLV2* gxfx_create_muff();
PluginLV2* gxfx_create_cstb();
PluginLV2* gxfx_create_aclipper();
PluginLV2* gxfx_create_mxrdist();
PluginLV2* gxfx_create_rangem();
PluginLV2* gxfx_create_mole();
PluginLV2* gxfx_create_hfb();
PluginLV2* gxfx_create_hogsfoot();
PluginLV2* gxfx_create_softclip();
PluginLV2* gxfx_create_bassbooster();
PluginLV2* gxfx_create_highbooster();
PluginLV2* gxfx_create_expander();
PluginLV2* gxfx_create_susta();

// EQ family (ids 28..33): gxfx_dsp_eq.cpp.
PluginLV2* gxfx_create_graphiceq();
PluginLV2* gxfx_create_selecteq();
PluginLV2* gxfx_create_tonecontroll();
PluginLV2* gxfx_create_moog();
PluginLV2* gxfx_create_low_high_pass();
PluginLV2* gxfx_create_noise_shaper();

// Wah family (ids 34..37): gxfx_dsp_wah.cpp.
PluginLV2* gxfx_create_wahmodel();
PluginLV2* gxfx_create_crybaby();
PluginLV2* gxfx_create_autowah();
PluginLV2* gxfx_create_dunwah();

// Modulation family (ids 38..44): gxfx_dsp_mod.cpp.
PluginLV2* gxfx_create_vibe();
PluginLV2* gxfx_create_tubetremelo();
PluginLV2* gxfx_create_tubevibrato();
PluginLV2* gxfx_create_switched_tremolo();
PluginLV2* gxfx_create_phaser_st();
PluginLV2* gxfx_create_flanger_st();
PluginLV2* gxfx_create_chorus_mono();

// Time/delay family (ids 45..56): gxfx_dsp_time.cpp.
PluginLV2* gxfx_create_duck_delay();
PluginLV2* gxfx_create_duck_delay_st();
PluginLV2* gxfx_create_digital_delay();
PluginLV2* gxfx_create_digital_delay_st();
PluginLV2* gxfx_create_gxtape();
PluginLV2* gxfx_create_gxtape_st();
PluginLV2* gxfx_create_gxechocat();
PluginLV2* gxfx_create_gxtubedelay();
PluginLV2* gxfx_create_ts9();
PluginLV2* gxfx_create_oc_2();
PluginLV2* gxfx_create_classic_delay();
PluginLV2* gxfx_create_classic_echo();

// Reverb family (ids 57..60): gxfx_dsp_reverb.cpp.
PluginLV2* gxfx_create_zita_rev1();
PluginLV2* gxfx_create_freeverb();
PluginLV2* gxfx_create_room_simulator();
PluginLV2* gxfx_create_shimmizita();

// Amp + tonestack family (ids 61..65, 75): gxfx_dsp_amps.cpp (the
// tonestack row consumes the TU's tonemodel aggregate factory).
PluginLV2* gxfx_create_ampmodel();
PluginLV2* gxfx_create_tonemodel();
PluginLV2* gxfx_create_studiopre();
PluginLV2* gxfx_create_alembic();
PluginLV2* gxfx_create_w20();
PluginLV2* gxfx_create_ampmodul();

// Multiband + utility family (ids 66..70, 72..74): gxfx_mb.cpp.
PluginLV2* gxfx_create_mbcompressor();
PluginLV2* gxfx_create_mbdelay();
PluginLV2* gxfx_create_mbdistortion();
PluginLV2* gxfx_create_mbecho();
PluginLV2* gxfx_create_barkgraphiceq();
PluginLV2* gxfx_create_balance();
PluginLV2* gxfx_create_outputlevel();
PluginLV2* gxfx_create_ampout();

// Effect id (gxfx_catalog.h row order) → factory. Rows 0..75.
// INCREMENTAL-PORT MODEL: the catalog (generated, 83 ids) is the complete
// menu; availability is honest per id. Ids 76..82 are NULL factories —
// their DSP TUs (cabinet/redeye/metalamp/metalhead convolver wrappers,
// detune, tuner, livelooper) have not been ported yet, so gxfx_create
// returns null, gxfx_effect_info reports available=0 and the engine's
// set_slot rejects them as "effect unavailable" (the frontend greys them
// out as "(not ported)"). Each port wave replaces a run of nulls with
// real factories — flip the rows here, nothing else; the catalog header
// and its JSON surface never change.
static const gxfx_factory GXFX_FACTORIES[GXFX_EFFECT_COUNT] = {
    gxfx_create_wah,         /*  0 wah            */
    gxfx_create_overdrive,   /*  1 overdrive      */
    gxfx_create_distortion,  /*  2 distortion     */
    gxfx_create_compressor,  /*  3 compressor     */
    gxfx_create_chorus,      /*  4 chorus         */
    gxfx_create_flanger,     /*  5 flanger        */
    gxfx_create_phaser,      /*  6 phaser         */
    gxfx_create_tremolo,     /*  7 tremolo        */
    gxfx_create_delay,       /*  8 delay          */
    gxfx_create_echo,        /*  9 echo           */
    gxfx_create_reverb,      /* 10 reverb         */
    gxfx_create_fuzzface,    /* 11 fuzzface       */
    gxfx_create_fuzzfacefm,  /* 12 fuzzfacefm     */
    gxfx_create_fumaster,    /* 13 fumaster       */
    gxfx_create_hornet,      /* 14 hornet         */
    gxfx_create_muff,        /* 15 muff           */
    gxfx_create_cstb,        /* 16 cstb           */
    gxfx_create_aclipper,    /* 17 aclipper       */
    gxfx_create_mxrdist,     /* 18 mxrdist        */
    gxfx_create_rangem,      /* 19 rangem         */
    gxfx_create_mole,        /* 20 mole           */
    gxfx_create_hfb,         /* 21 hfb            */
    gxfx_create_hogsfoot,    /* 22 hogsfoot       */
    gxfx_create_softclip,    /* 23 softclip       */
    gxfx_create_bassbooster, /* 24 bassbooster    */
    gxfx_create_highbooster, /* 25 highbooster    */
    gxfx_create_expander,    /* 26 expander       */
    gxfx_create_susta,       /* 27 susta          */
    gxfx_create_graphiceq,   /* 28 graphiceq      */
    gxfx_create_selecteq,    /* 29 selecteq       */
    gxfx_create_tonecontroll,/* 30 tonecontroll   */
    gxfx_create_moog,        /* 31 moog           */
    gxfx_create_low_high_pass, /* 32 low_high_pass */
    gxfx_create_noise_shaper,/* 33 noise_shaper   */
    gxfx_create_wahmodel,    /* 34 wahmodel       */
    gxfx_create_crybaby,     /* 35 crybaby        */
    gxfx_create_autowah,     /* 36 autowah        */
    gxfx_create_dunwah,      /* 37 dunwah         */
    gxfx_create_vibe,        /* 38 vibe           */
    gxfx_create_tubetremelo, /* 39 tubetremelo    */
    gxfx_create_tubevibrato, /* 40 tubevibrato    */
    gxfx_create_switched_tremolo, /* 41 switched_tremolo */
    gxfx_create_phaser_st,   /* 42 phaser_st      */
    gxfx_create_flanger_st,  /* 43 flanger_st     */
    gxfx_create_chorus_mono, /* 44 chorus_mono    */
    gxfx_create_duck_delay,  /* 45 duck_delay     */
    gxfx_create_duck_delay_st, /* 46 duck_delay_st */
    gxfx_create_digital_delay, /* 47 digital_delay */
    gxfx_create_digital_delay_st, /* 48 digital_delay_st */
    gxfx_create_gxtape,      /* 49 gxtape         */
    gxfx_create_gxtape_st,   /* 50 gxtape_st      */
    gxfx_create_gxechocat,   /* 51 gxechocat      */
    gxfx_create_gxtubedelay, /* 52 gxtubedelay    */
    gxfx_create_ts9,         /* 53 ts9            */
    gxfx_create_oc_2,        /* 54 oc_2           */
    gxfx_create_classic_delay, /* 55 classic_delay */
    gxfx_create_classic_echo, /* 56 classic_echo   */
    gxfx_create_zita_rev1,   /* 57 zita_rev1      */
    gxfx_create_freeverb,    /* 58 freeverb       */
    gxfx_create_room_simulator, /* 59 room_simulator */
    gxfx_create_shimmizita,  /* 60 shimmizita     */
    gxfx_create_ampmodel,    /* 61 ampmodel       */
    gxfx_create_tonemodel,   /* 62 tonestack      */
    gxfx_create_studiopre,   /* 63 studiopre      */
    gxfx_create_alembic,     /* 64 alembic        */
    gxfx_create_w20,         /* 65 w20            */
    gxfx_create_mbcompressor, /* 66 mbcompressor  */
    gxfx_create_mbdelay,     /* 67 mbdelay        */
    gxfx_create_mbdistortion, /* 68 mbdistortion  */
    gxfx_create_mbecho,      /* 69 mbecho         */
    gxfx_create_barkgraphiceq, /* 70 barkgraphiceq */
    gxfx_create_bigmuffpi,   /* 71 bigmuffpi      */
    gxfx_create_balance,     /* 72 balance        */
    gxfx_create_outputlevel, /* 73 outputlevel    */
    gxfx_create_ampout,      /* 74 ampout         */
    gxfx_create_ampmodul,    /* 75 ampmodul       */
    /* 76..82: not yet ported (null factories — see the comment above) */
    0, /* 76 cabinet        */ 0, /* 77 redeye         */
    0, /* 78 metalamp       */ 0, /* 79 metalhead      */
    0, /* 80 detune         */ 0, /* 81 tuner          */
    0, /* 82 livelooper     */
};

static bool gxfx_id_ok(int fx)
{
    return fx >= 0 && fx < GXFX_EFFECT_COUNT;
}

// One DSP instance: the plugin pointer and nothing else. Opaque to Rust.
struct GxFxInstance {
    PluginLV2* plugin;
};

/*
 * C ABI for the Rust side. Catalog queries are pure static-data reads; the
 * lifecycle contract is: gxfx_create (calls set_samplerate BEFORE any port
 * connection — set_samplerate's init() computes the rate-dependent
 * constants; for chorus/delay/echo the big delay buffers are new'd in
 * activate(true)) → one gxfx_connect_port per param (port id from
 * gxfx_param_def; storage owned by the caller) → gxfx_activate(true) →
 * gxfx_run_mono/gxfx_run_stereo per block (in-place) → gxfx_activate(false)
 * / gxfx_destroy (deactivate frees the lazily-allocated buffers before
 * delete_instance).
 */
extern "C" {

int gxfx_effect_count(void)
{
    return GXFX_EFFECT_COUNT;
}

int gxfx_param_count(int fx)
{
    return gxfx_id_ok(fx) ? GXFX_CATALOG[fx].param_count : 0;
}

int gxfx_is_stereo(int fx)
{
    return gxfx_id_ok(fx) ? GXFX_CATALOG[fx].stereo : 0;
}

float gxfx_param_default(int fx, int param)
{
    if (!gxfx_id_ok(fx) || param < 0 || param >= GXFX_CATALOG[fx].param_count)
        return 0.0f;
    return GXFX_CATALOG[fx].params[param].def;
}

/* Full param definition by (fx, param index); null when out of range. The
 * pointee is static storage (gxfx_catalog.h) valid for the object lifetime. */
const GxFxParamDef* gxfx_param_def(int fx, int param)
{
    if (!gxfx_id_ok(fx) || param < 0 || param >= GXFX_CATALOG[fx].param_count)
        return 0;
    return &GXFX_CATALOG[fx].params[param];
}

/* Effect-level metadata + AVAILABILITY: fills *out with effect fx's key /
 * label / category / stereo / memory / param_count (static catalog
 * storage) plus available = (factory != null) — ids whose DSP TU has not
 * been ported yet ride null factories and report available 0. Returns 1
 * on success, 0 on an unknown id or null out. */
int gxfx_effect_info(int fx, GxFxEffectInfo* out)
{
    if (!gxfx_id_ok(fx) || !out)
        return 0;
    const GxFxEffectDef* e = &GXFX_CATALOG[fx];
    out->key = e->key;
    out->label = e->label;
    out->category = e->category;
    out->stereo = e->stereo;
    out->memory = e->memory;
    out->param_count = e->param_count;
    out->available = GXFX_FACTORIES[fx] != 0;
    return 1;
}

/* Create a DSP instance for effect id at the given rate; null when the id
 * is unknown or allocation fails. Calls set_samplerate BEFORE any port
 * connection. */
GxFxInstance* gxfx_create(int fx, uint32_t sample_rate)
{
    if (!gxfx_id_ok(fx))
        return 0;
    gxfx_factory factory = GXFX_FACTORIES[fx];
    if (!factory)
        return 0;
    PluginLV2* p = factory();
    if (!p)
        return 0;
    if (p->set_samplerate)
        p->set_samplerate(sample_rate, p);
    GxFxInstance* h = (GxFxInstance*)malloc(sizeof(GxFxInstance));
    if (!h) {
        if (p->delete_instance)
            p->delete_instance(p);
        return 0;
    }
    h->plugin = p;
    return h;
}

/* Connect ONE control port (LV2 PortIndex) to caller-owned float storage.
 * 1 on success, 0 on bad handle/port. */
int gxfx_connect_port(GxFxInstance* handle, int port, float* storage)
{
    if (!handle || !handle->plugin || !handle->plugin->connect_ports || !storage)
        return 0;
    handle->plugin->connect_ports((uint32_t)port, storage, handle->plugin);
    return 1;
}

/* activate(true)/deactivate(false); 0 only on a bad handle. The plugin's
 * activate_plugin return value is NOT a success code — the generated
 * activate() returns a constant 0 (guitarix/OctOBX hosts ignore it too) —
 * so it is deliberately not propagated. */
int gxfx_activate(GxFxInstance* handle, int start)
{
    if (!handle || !handle->plugin)
        return 0;
    if (handle->plugin->activate_plugin)
        handle->plugin->activate_plugin(start != 0, handle->plugin);
    return 1;
}

/* One block, in place: buf→buf via the plugin's mono_audio member. */
void gxfx_run_mono(GxFxInstance* handle, float* buf, int n)
{
    if (!handle || !handle->plugin || !handle->plugin->mono_audio || !buf || n <= 0)
        return;
    handle->plugin->mono_audio(n, buf, buf, handle->plugin);
}

/* One block, in place: l→l, r→r via the plugin's stereo_audio member. */
void gxfx_run_stereo(GxFxInstance* handle, float* l, float* r, int n)
{
    if (!handle || !handle->plugin || !handle->plugin->stereo_audio || !l || !r || n <= 0)
        return;
    handle->plugin->stereo_audio(n, l, r, l, r, handle->plugin);
}

/* deactivate (frees the lazily-new'd delay/reverb buffers) + delete + free. */
void gxfx_destroy(GxFxInstance* handle)
{
    if (!handle)
        return;
    PluginLV2* p = handle->plugin;
    if (p) {
        if (p->activate_plugin)
            p->activate_plugin(false, p);
        if (p->delete_instance)
            p->delete_instance(p);
    }
    free(handle);
}

} /* extern "C" */
