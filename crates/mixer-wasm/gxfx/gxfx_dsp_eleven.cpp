// gxfx_dsp_eleven.cpp — ten guitarix faust DSP effects as freestanding
// object code for the CakeMix mixer engine (crates/mixer-wasm).
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
// gx_resample/zita (resampler) and distortion rides its own TU,
// gxfx_dsp_distortion.cpp. THIS is the only TU that may define the global
// operator new/delete overloads and memset (there is no C++ runtime on the
// freestanding target — they route to C malloc/free); gxfx_dsp_distortion.cpp
// must not.
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
// separate TU: it alone pulls gx_resampler/zita link-complete).
PluginLV2* gxfx_create_distortion();

// Effect id (gxfx_catalog.h row order) → factory. Rows 0..10.
static const gxfx_factory GXFX_FACTORIES[GXFX_EFFECT_COUNT] = {
    gxfx_create_wah,         /*  0 wah         */
    gxfx_create_overdrive,   /*  1 overdrive   */
    gxfx_create_distortion,  /*  2 distortion  */
    gxfx_create_compressor,  /*  3 compressor  */
    gxfx_create_chorus,      /*  4 chorus      */
    gxfx_create_flanger,     /*  5 flanger     */
    gxfx_create_phaser,      /*  6 phaser      */
    gxfx_create_tremolo,     /*  7 tremolo     */
    gxfx_create_delay,       /*  8 delay       */
    gxfx_create_echo,        /*  9 echo        */
    gxfx_create_reverb,      /* 10 reverb      */
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
