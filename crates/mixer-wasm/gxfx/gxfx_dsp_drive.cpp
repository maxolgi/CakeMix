// gxfx_dsp_drive.cpp — the drive + dynamics families (gxfx catalog ids
// 11..27) as freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm).
//
// Provenance: productionized port of OctOBX wasm/obxd/gxfx_dsp_drive.cpp
// (the authoritative wrapper TU). Each guitarix faust class is pulled
// into its own namespace together with its PortIndex enum, transcribed
// VERBATIM from OctOBX — the enum values encode the LV2 ttl port
// indexes of the gx_<fx>.lv2 bundles (cross-checked against the
// gxfx_catalog.h rows for ids 11..27) and a wrong enum silently routes
// params to the wrong controls. Factory order matches the manifest
// order (tools/gen-gxfx-params.mjs / OctOBX src/gxfx-params.ts rows
// 11..27): fuzzface, fuzzfacefm, fumaster, hornet, muff, cstb,
// aclipper, mxrdist, rangem, mole, hfb, hogsfoot (drive), softclip,
// bassbooster, highbooster (orphan boosters) and expander, susta
// (dynamics).
//
// Adapted from OctOBX, same rules as gxfx_dsp_distortion.cpp:
//  - all guitarix sources are included via RELATIVE paths from this
//    dir into ../../../vendor/guitarix/trunk/... so build.rs needs no
//    new include dirs;
//  - the tube transfer tables (trany.h + its KT88/7199P data) are
//    staged at GLOBAL scope below — see the staging block comment;
//  - muff.cc / aclipper.cc oversample via gx_resample::
//    FixedRateResampler, but this TU pulls the HEADER only: the
//    implementation (gx_resampler.cc + zita resampler.cc /
//    resampler-table.cc) is compiled link-complete into
//    gxfx_dsp_distortion.cpp's TU (OctOBX keeps it in gxfx_dsp.cpp's
//    TU; same single-owner rule) and resolves at archive link time.
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete
// overloads and the memset definition for the whole gxfx archive (the
// classes' new/delete here resolve against them at archive link time);
// this TU defines NONE of them, no extern "C", no emscripten. This TU
// IS the single owner of the trany tube tables (tubetable_*, tranytab,
// tranytab2) — no other gxfx TU may stage them.
//
// Statelessness: all param storage and strip state live on the Rust
// side; this is only the DSP classes + factories. Ids 11..27 stay
// unavailable (null factories in gxfx_dsp_eleven.cpp's table) until a
// follow-up task flips those rows — this TU enters the archive
// unlinked until then.

#include "gxfx_prelude.h"
#include <math.h> // C-scope libm: clipping.h (aclipper/mxrdist) calls bare
                  // copysign() and includes no math header itself — it rides
                  // the includer's declarations. The distortion TU gets this
                  // transitively from the zita .cc files it compiles; this TU
                  // pulls the resampler HEADER only, so it must include
                  // <math.h> itself (the wasm shim declares copysign there).
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/gx_pluginlv2.h"

// ---- trany.h staging (build glue — inline, no new shim file) ----
// fuzzface, fuzzfacefm, fumaster, muff, cstb and susta call
// Ftrany()/Rtrany() on the tube transfer tables. Upstream trany.h
// pulls its data with a bare `#include "KT88.cc"` / `#include
// "7199P.cc"` from DSP/tube_tables/, which is deliberately NOT on
// build.rs's include set — a quoted include of trany.h cannot resolve
// them (the search covers DSP/, then this TU's dir, then the -I dirs;
// none contain the tables), so the header cannot be included as-is.
// Instead its content is staged here VERBATIM except for the two table
// includes, which use explicit relative paths into tube_tables/ (the
// data itself stays vendored, never copied). Defining the vendor guard
// up front neuters each faust .cc's own `#include "trany.h"` — the
// same include-once trick OctOBX relies on, just with the guard set
// before the first include instead of after the global-scope one — so
// the classes see ::Ftrany/::Rtrany and the TRANY_TABLE_* enumerators
// via unqualified lookup from inside their namespaces.
#define SRC_HEADERS_TRANY_H_

struct table1d { // 1-dimensional function table
    float low;
    float high;
    float istep;
    int size;
    float data[];
};

template <int tab_size>
struct table1d_imp {
    float low;
    float high;
    float istep;
    int size;
    float data[tab_size];
    operator table1d&() const { return *(table1d*)this; }
};

#include "../../../vendor/guitarix/trunk/src/LV2/DSP/tube_tables/KT88.cc"
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/tube_tables/7199P.cc"

enum {
    TRANY_TABLE_KT88_68k,
    TRANY_TABLE_KT88_250k,
    TRANY_TABLE_7199P_68k,
    TRANY_TABLE_7199P_250k,
    TRANY_TABLE_SIZE
};

table1d *tranytab[TRANY_TABLE_SIZE] = {
    &static_cast<table1d&>(tubetable_KT88[0]),
    &static_cast<table1d&>(tubetable_KT88[1]),
    &static_cast<table1d&>(tubetable_7199P[0]),
    &static_cast<table1d&>(tubetable_7199P[1]),
};

table1d *tranytab2[TRANY_TABLE_SIZE] = {
    &static_cast<table1d&>(tubetable2_KT88[0]),
    &static_cast<table1d&>(tubetable2_KT88[1]),
    &static_cast<table1d&>(tubetable2_7199P[0]),
    &static_cast<table1d&>(tubetable2_7199P[1]),
};

static inline double Ftrany(int table, double Vgk) {
    const table1d& tab = *tranytab[table];
    double f = (Vgk - tab.low) * tab.istep;
    int i = static_cast<int>(f);
    if (i < 0)
        return tab.data[0];
    if (i >= tab.size-1)
        return tab.data[tab.size-1];
    f -= i;
    return tab.data[i]*(1-f) + tab.data[i+1]*f;
}

static inline double Rtrany(int table, double Vgk) {
    const table1d& tab = *tranytab2[table];
    double f = (Vgk - tab.low) * tab.istep;
    int i = static_cast<int>(f);
    if (i < 0)
        return tab.data[0];
    if (i >= tab.size-1)
        return tab.data[tab.size-1];
    f -= i;
    return tab.data[i]*(1-f) + tab.data[i+1]*f;
}

// muff.cc / aclipper.cc oversample their clipping stage via
// gx_resample::FixedRateResampler. Header only — the implementation is
// compiled into gxfx_dsp_distortion.cpp's TU and links from there
// (muff.cc relies on the host TU for this include, exactly as in
// OctOBX).
#include "../../../vendor/guitarix/trunk/src/LV2/DSP/gx_resampler.h"

namespace gxfx_fuzzface {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, FUZZ, LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/fuzzface.cc"
}

namespace gxfx_fuzzfacefm {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, DRIVE, FUZZ, AUDIO_IN, LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/fuzzfacefm.cc"
}

namespace gxfx_fumaster {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, TONE, VOLUME, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/fumaster.cc"
}

namespace gxfx_hornet {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, FUZZ, SUSTAIN, VOLUME } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/hornet.cc"
}

namespace gxfx_muff {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, TONE, VOLUME } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/muff.cc"
}

namespace gxfx_cstb {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, ATTACK, LEVEL, WET_DRY, BYPASS } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/cstb.cc"
}

namespace gxfx_aclipper {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, DRIVE, LEVEL, TONE, BYPASS } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/aclipper.cc"
}

namespace gxfx_mxrdist {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, VOLUME, DRIVE } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/mxrdist.cc"
}

namespace gxfx_rangem {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, BOOST, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/rangem.cc"
}

namespace gxfx_mole {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, BOOST, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/mole.cc"
}

namespace gxfx_hfb {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, INTENSITY, VOLUME } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/hfb.cc"
}

namespace gxfx_hogsfoot {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, VOLUME, WET_DRY } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/hogsfoot.cc"
}

// --- orphans (no .lv2 bundle; param enums from the .cc PortIndex comment) ---

namespace gxfx_softclip {
typedef enum { FUZZ } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/softclip.cc"
}

namespace gxfx_bassbooster {
typedef enum { LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/bassbooster.cc"
}

namespace gxfx_highbooster {
typedef enum { LEVEL } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/highbooster.cc"
}

// --- dynamics ---

namespace gxfx_expander {
typedef enum { RATIO, KNEE, THRESHOLD, RELEASE, ATTACK, EFFECTS_OUTPUT, EFFECTS_INPUT } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/expander.cc"
}

namespace gxfx_susta {
typedef enum { EFFECTS_OUTPUT, EFFECTS_INPUT, SUSTAIN, VOLUME } PortIndex;
#include "../../../vendor/guitarix/trunk/src/LV2/faust-generated/susta.cc"
}

// namespace paths: gxfx_fuzzface::fuzzface::plugin() etc. Factory order
// MUST match the generator manifest order (ids 11..27).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_fuzzface() { return gxfx_fuzzface::fuzzface::plugin(); }
PluginLV2* gxfx_create_fuzzfacefm() { return gxfx_fuzzfacefm::fuzzfacefm::plugin(); }
PluginLV2* gxfx_create_fumaster() { return gxfx_fumaster::fumaster::plugin(); }
PluginLV2* gxfx_create_hornet() { return gxfx_hornet::hornet::plugin(); }
PluginLV2* gxfx_create_muff() { return gxfx_muff::muff::plugin(); }
PluginLV2* gxfx_create_cstb() { return gxfx_cstb::cstb::plugin(); }
PluginLV2* gxfx_create_aclipper() { return gxfx_aclipper::aclipper::plugin(); }
PluginLV2* gxfx_create_mxrdist() { return gxfx_mxrdist::mxrdist::plugin(); }
PluginLV2* gxfx_create_rangem() { return gxfx_rangem::rangem::plugin(); }
PluginLV2* gxfx_create_mole() { return gxfx_mole::mole::plugin(); }
PluginLV2* gxfx_create_hfb() { return gxfx_hfb::hfb::plugin(); }
PluginLV2* gxfx_create_hogsfoot() { return gxfx_hogsfoot::hogsfoot::plugin(); }
PluginLV2* gxfx_create_softclip() { return gxfx_softclip::softclip::plugin(); }
PluginLV2* gxfx_create_bassbooster() { return gxfx_bassbooster::bassbooster::plugin(); }
PluginLV2* gxfx_create_highbooster() { return gxfx_highbooster::highbooster::plugin(); }
PluginLV2* gxfx_create_expander() { return gxfx_expander::expander::plugin(); }
PluginLV2* gxfx_create_susta() { return gxfx_susta::susta::plugin(); }
