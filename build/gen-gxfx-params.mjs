#!/usr/bin/env node
// ===========================================================================
// gen-gxfx-params.mjs — generates the guitarix FX catalog header for the
// CakeMix mixer engine (crates/mixer-wasm/gxfx/gxfx_catalog.h).
//
// Port of OctOBX's tools/gen-gxfx-params.mjs (same manifest order, same
// parsing rules — CakeMix vendors the SAME guitarix tree at the SAME pin
// a4c561a, so the parsing runs identically). CakeMix's generator emits
// ONLY the C catalog header; the effect-level availability flag lives in
// gxfx_dsp_eleven.cpp's factory table, not here.
//
// SOURCE OF TRUTH for param DATA, per effect kind:
//   - v1 eleven (wah..reverb, ids 0..10): PINNED in
//     build/gxfx-param-spec.json — the ports, symbols, labels, ranges and
//     defaults were transcribed from the guitarix ttl files and hand-verified
//     when the 11-effect rack shipped (the labels are humanized vs the raw
//     ttl "VOLUME"/"WAH" strings, so the ttl cannot re-produce them). The
//     generator reads the first 11 spec entries back verbatim —
//     byte-identical labels/values, forever.
//   - NEW effects: parsed straight from the pinned guitarix submodule —
//     bundle ttl files (`ttl:` manifest entries: port symbols / ranges /
//     defaults from vendor/guitarix/trunk/src/LV2/gx_*.lv2/*.ttl, labels
//     humanized to Title Case), orphan faust classes (`orphan:`
//     entries: param metadata from the .cc connect_ports comments
//     `// , default, min, max, step` in faust-generated/), or hand-authored
//     param rows (`params:` entries: for DSP with no parseable ttl — e.g.
//     gxautowah.lv2 lists TWO plugins in one ttl so the first-block parser
//     cannot reach the second, and its auto variant has no control params
//     at all; inline entries are the ONLY kind allowed zero params).
//
// MANIFEST: the effect catalog below is the single id-order list — effect
// ids are the manifest index (existing 0..10 stable, new appended in
// manifest order, all 83 ids). gxfx_dsp_eleven.cpp's GXFX_FACTORIES table
// mirrors this order; null factories mark not-yet-ported DSP (the engine
// reports those effects unavailable).
//
// Port-exposure policy (both sources):
//   - audio ports are never params; stereo := ttl audio-input count >= 2
//   - control INPUT ports are params, EXCEPT `lv2:designation lv2:enabled`
//     (wrapper-level BYPASS — the faust classes ignore it; matches v1, where
//     bossds1's ttl BYPASS port is not exposed), `pprop:trigger` (tremolo
//     reset style) and `pprop:notOnGUI` ports.
//   - control OUTPUT ports (meters) are parsed (spec out_ports) but NOT
//     emitted — CakeMix's engine has no out-port surface yet.
//   - ttl carries no step: derived deterministically (1 for integer/toggled,
//     else by range span: <=3 -> 0.01, <=30 -> 0.1, <=300 -> 1, else 10).
//     Step is display metadata only — the 0..1<->engine transforms are linear.
//
// Memory-class flag (per effect, emitted as GxFxEffectDef.memory): effects
// whose DSP allocates MB-scale buffers at create/first-activate — the
// delay/time family (category 'delay'), the reverb family ('reverb'), and
// (for later waves) the convolver-based cabinet/speaker sims plus the live
// looper (64 MiB tapes). Everything else is false — including detune's
// fixed ~840 KB frame arrays (sub-MB). The Rust memory-cap check consults
// this flag; behavior for the live eleven is identical to the old
// hardcoded id list (delay/echo/reverb flagged, others not).
//
// Emitted artifact (committed; deterministic, no timestamps):
//   crates/mixer-wasm/gxfx/gxfx_catalog.h — the engine catalog header:
//   constants + GxFxParamDef/GxFxEffectDef/GxFxEffectInfo + per-effect
//   param tables + GXFX_CATALOG + padded FX_DEFAULTS/FX_PORTS tables.
//
// Usage:
//   node build/gen-gxfx-params.mjs           # write the header
//   node build/gen-gxfx-params.mjs --check   # byte-compare against the
//                                            # committed header; exit 0
//                                            # fresh, exit 1 with a diff
//
// Deterministic by construction: effects in manifest (id) order, params in
// port order, insertion-stable object construction — --check is
// byte-stable across runs. Node >= 20, no dependencies.
// ===========================================================================

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));

const SPEC_JSON = 'build/gxfx-param-spec.json';
const OUT_H = 'crates/mixer-wasm/gxfx/gxfx_catalog.h';

const GX_LV2 = 'vendor/guitarix/trunk/src/LV2';
const GX_FAUST = `${GX_LV2}/faust-generated`;

const FX_SLOTS = 11;
const FX_SLOT_PARAMS = 48;  // per-slot param array size (widest effect: 39 — asserted below)
// PortIndex ceiling for the shared validation below. Ports are only ever
// FORWARDED to the DSP's connect_ports() (never used as array indices), so
// the bound is a sanity check, not a layout constraint — but it must
// exceed the widest wrapper port space (livelooper's synct sits at ttl
// port 48, past FX_SLOT_PARAMS).
const FX_PORT_MAX = 256;
// Widest param count the manifest may produce (reported at generation
// time; a hard assert below fails the run if it ever exceeds the padded
// row width). OctOBX's generator observed 39 (livelooper).
const FX_MAX_LIVE_PARAMS = 39;

// Frozen menu-category vocabulary (mirrors OctOBX src/gxfx-params.ts
// FxCategory and the mixer dropdown grouping).
const CATEGORY_VOCAB = new Set([
    'wah', 'drive', 'eq', 'filter', 'dynamics', 'modulation',
    'delay', 'reverb', 'amp', 'tonestack', 'multiband', 'special', 'utility',
]);

// v1 effect keys in canonical id order 0..10 — their param data is pinned in
// the committed spec JSON (see header). Category assignments kept from v1.
const V1_CATEGORY_BY_KEY = {
    wah: 'wah',
    overdrive: 'drive',
    distortion: 'drive',
    compressor: 'dynamics',
    chorus: 'modulation',
    flanger: 'modulation',
    phaser: 'modulation',
    tremolo: 'modulation',
    delay: 'delay',
    echo: 'delay',
    reverb: 'reverb',
};
const V1_KEYS = Object.keys(V1_CATEGORY_BY_KEY);

// Memory-class keys outside the delay/reverb categories (see the header
// comment): the convolver-based cab/speaker sims (later waves) and the
// live looper's 4 x 16 MiB tapes.
const MEMORY_CLASS_KEYS = new Set(['cabinet', 'redeye', 'metalamp', 'metalhead', 'livelooper']);
const isMemoryClass = (fx) =>
    fx.category === 'delay' || fx.category === 'reverb' || MEMORY_CLASS_KEYS.has(fx.key);

// ---------------------------------------------------------------------------
// MANIFEST — the effect catalog in id order (index = effect id).
// v1 entries: { key, pin: true, category } — data from the spec JSON.
// New entries: { key, menuName, category, ttl } parses the bundle ttl
// relative to vendor/guitarix/trunk/src/LV2/, or
// { key, menuName, category, orphan } parses the faust-generated .cc
// connect_ports comments (classes with no .lv2 bundle). Optional `stereo:
// true` overrides the derived flag — needed for orphan classes whose faust
// process is natively 2-in/2-out (tonecontroll, moog expose only
// stereo_audio; running them dual-mono would find mono_audio == 0 and
// silently do nothing).
// Phase 1-a ships the drive family (fuzzes/distortions/boosters + the
// softclip orphan + the booster halves of the gxbooster composite) and the
// remaining dynamics bundles. Excluded per fx2plan.md: gx_fuzz (wrapper
// composite: bmfp+lowpass_up+lowpass_down+noiser), gx_distortion / gx_feedback
// orphans (plan defaults optional-skip), mbdistortion/mbcompressor (multiband
// family, later phase), jcm800pre (Phase 4, needs Eigen).
// Phase 1-b ships the eq family. Excluded per fx2plan.md: biquad orphan
// (plan optional-skip), gx_barkgraphiceq (multiband family, later phase),
// gxtilttone (tilt-tone preamp hybrid with a drive stage — not in the plan's
// eq enumeration; add with the amp family if wanted). No `gx_eq` 10-band
// bundle exists in the pinned submodule — graphiceq is the tree's only
// standalone graphic EQ.
// Phase 1-c ships the wah family: the first HOST-SIDE AGGREGATE ("Wah
// Model" — gx_colwah.lv2's 7 wah model classes behind one MODEL param; the
// ttl describes the full aggregate surface so it parses like any bundle,
// while the DSP wave hot-swaps the underlying instance), the crybaby
// orphan, and both gxautowah.lv2 variants from the bundle-local
// dunwahauto.cc (envelope-driven auto + manual; inline params — see the
// header "SOURCE OF TRUTH"). Also present in faust-generated/ but NOT
// shipped: colbwah / jenbasswah / rolwah (not in wah.h's 7-model set nor in
// the fx2plan orphan enumeration) and the faust autowah.cc orphan (superseded
// by the bundle's circuit-modelled dunwahauto, per fx2plan "use gxautowah
// bundle's local dunwahauto.cc"). low_high_cut.cc is a wrapper companion
// (skip per fx2plan).
// Phase 1-d ships the modulation family: the bundle-local gx_vibe.lv2
// vibe.cc STEREO Uni-Vibe-style class (the file also has plugin_mono(); the
// mono variant stays unshipped), the 12ax7-table gxtubetremelo + gxtubevibrato
// bundles, the gx_switched_tremolo bundle (multi-step stepped tremolo), and
// three classic faust orphans — phaser.cc (stereo 10-param, distinct from
// v1's mono phaser id 6), flanger.cc (stereo, distinct from v1's gx_flanger
// id 5) and chorus_mono.cc (mono, v1's chorus id 4 is the stereo class).
// Also present but NOT shipped here: flanger_mono.cc / phaser_mono.cc
// (phaser_mono IS v1 id 6; flanger_mono duplicates the shipped classic
// flanger in mono), gx_ampmodul.cc (fx2plan assigns it to the later
// "utility" additions), gx_vibrochump.cc (a chump preamp — Phase 2
// convolver redeye family).
// Phase 1-e ships the time/delay family: the bundle-LOCAL duck_delay(.st)
// and digital_delay(.st) .cc classes (digital_delay's fVec2[524288] double
// = 4 MB per object at CREATION — a slot costs 8 MB dual-mono / 4 MB
// stereo; MEMORY-CLASS flagged), the 12au7-table gxtape(.st) +
// copicat-table gxechocat + 12ax7-table gxtubedelay faust classes, the
// circuit-modelled gxts9 (ts9sim+ts9nonlin, menu category DRIVE — a
// Tubescreamer-style overdrive, not a delay), the gx_oc_2 octave divider
// (+triggers_logic.h; menu category SPECIAL — the frozen vocabulary has no
// pitch group and "special" is where fx2plan parks non-insert-shaped
// effects), and the classic mono delay.cc/echo.cc orphans ("Classic
// Delay"/"Classic Echo" — v1 ids 8/9 are the STEREO stereodelay/stereoecho
// classes). digital_delay(.st) ttl ports SYNC + HOSTBPM are wrapper-level
// host-tempo-sync controls the DSP class ignores — skipped via skipPorts,
// like BYPASS. Also present but NOT shipped here: reverse delay/echo do
// not exist in the tree (no other true delay/echo/tape bundles remain —
// gx_delay.lv2 and gx_echo.lv2 are wrapper-only bundles over
// stereodelay.cc/stereoecho.cc = v1 ids 8/9, and mbdelay/mbecho are the
// multiband family, later phase).
// Phase 1-f ships the reverb family: the faust-generated gx_zita_rev1
// (STANDALONE stereo class — nothing to do with v1's id 10 "Reverb", which
// is the stereoverb.cc class behind the gx_reverb.lv2 wrapper-only bundle),
// the freeverb orphan (mono classic Schroeder reverb), and the bundle-LOCAL
// room_simulator.cc + shimmizita.cc (plain faust classes, NOT
// convolver-based — fx2plan research). All four carry multi-MB static
// delay-line double arrays as class members (shimmizita ~4.8 MB, zita
// ~1.8 MB, room_simulator ~1.9 MB per object — allocated at CREATION like
// digital_delay's fVec2, bounded per slot; MEMORY-CLASS flagged). Also
// present but NOT shipped: gx_mbreverb.lv2/mbreverb~old.cc is a dead file
// (not built upstream) and faust-generated/mbreverb.cc is the multiband
// family (later phase); tonestack_ampeg_rev(.cc/_stereo.cc) are TONESTACK
// classes (amp family), the "rev" is Ampeg's model name, not a reverb;
// impulseresponse.cc is the Phase-2 convolver orphan.
// Phase 1-g ships the amp + tonestack family (one TU with valve.h included
// ONCE at global scope so the 14 tube_tables/*.cc files (~2.2 MB) are
// shared by all 19 gxamp namespaces): the "Amp Model" aggregate (19 mono
// gxamp classes behind MODEL 0..18 — gxamp.cc + gxamp2..18.cc + gxnoamp.cc
// at 18, order == upstream amp_model[]; the wrapper-level tonestack +
// convolver cab stages are NOT ported, cabs are Phase 2), the "Tone Stack"
// aggregate (27 STEREO tonestack classes behind MODEL 0..26, order ==
// upstream tonestack_model[]), and three preamps: studiopre (the STEREO
// gx_studiopre_st class — its mono sibling shares the guard-less alembic_*
// circuit tables, so only one variant can ride the TU), alembic (no
// tables), w20 (own w20 tables). gxtilttone is NOT shipped: fx2plan's
// composite default (its LV2 wrapper chains noiser.cc; the amp-family
// enumeration excludes it). Also present but NOT shipped: gxampN_stereo.cc
// classes (the mono wrapper is the canonical aggregate; shipping both would
// double the TU's table-linked classes for no menu gain), gx_chump/
// gx_bigchump/gx_vibrochump (Phase 2 redeye convolver family),
// gxmetal_amp/gxmetal_head (Phase 2 cab_data convolution),
// gx_preamp.cc (references gx_head engine headers), jcm800pre (Phase 4,
// Eigen).
// Phase 1-h ships the multiband + utility family — the LAST Phase 1
// family. Multiband: the four gx_mb* bundles over the faust mbc/mbdel/
// mbd/mbe classes (mbcompressor is the tree's 2nd-largest effect at 34
// params) and gx_barkgraphiceq (24-band bark-scale EQ over the
// bundle-LOCAL barkgraphiceq.cc + orfanidis_eq.h + bark_freq_grid.h). All
// five carry meter/bar OUTPUT ports (parsed into out_ports, parked on
// TU-local scratch by the factories — graphiceq precedent; NEVER params).
// NO gx_mbclipper exists in the tree; gx_mbreverb.lv2/mbreverb~old.cc is a
// dead file not built upstream (faust-generated/mbreverb.cc = Phase 2 per
// fx2plan). Utility orphans: balance, gx_outputlevel (NOT the _ladspa
// variant — that one is only referenced by src/ladspa/ladspa_guitarix.cpp;
// the LV2/v1-era tree uses gx_outputlevel.cc), gx_ampout (same rule), and
// gx_ampmodul ("Postamp", fx2plan's utility assignment;
// amplitude-modulation-adjacent tube postamp with dry/wet feedback paths).
// The closing enumeration audit (ls of trunk/src/LV2/ + the
// faust-generated orphans, cross-checked against the manifest) found
// exactly ONE real working insert the plan lists had missed: gx_bmp
// (GxBigMuffPi) — shipped here. Everything else still unshipped is
// deliberately left: Phase 2 (convolver/FFT: gx_cabinet, gx_redeye+chumps,
// gxmetal_amp/head, gx_detune, gxtuner, impulseresponse.cc orphan),
// Phase 3 (gx_livelooper), Phase 4 (gx_jcm800pre(_st), Eigen),
// plan-default composite skips (gx_fuzz = bmfp+lowpass_up+lowpass_down+
// noiser chain, gxbooster = bassbooster+highbooster — shipped as its
// parts, gxtilttone = tone+noiser chain), optional-skips (biquad,
// gx_distortion, gx_feedback orphans; gxfeed is not in the plan's
// enumeration either), wrapper companions (noiser/stereo_noiser,
// low_high_cut, uniBar), duplicates of shipped variants
// (gx_outputlevel_ladspa/gx_ampout_ladspa LADSPA builds, gxampN_stereo,
// tonestack mono variants, colbwah/jenbasswah/rolwah outside wah.h's
// 7-model set, flanger_mono, faust autowah.cc superseded by the bundle's
// dunwahauto), dead/unreferenced files (gx_mbreverb.lv2/mbreverb~old.cc not
// built upstream; faust mbreverb.cc referenced only by it), gx_preamp.cc
// (references gx_head engine headers), gx_amp_stereo (wrapper-only over the
// canonical mono gxamp).
// ---------------------------------------------------------------------------
const MANIFEST = [
    // --- v1 eleven (ids 0..10) — pinned data, canonical default chain ---
    ...V1_KEYS.map((key) => ({ key, pin: true, category: V1_CATEGORY_BY_KEY[key] })),

    // --- Phase 1-a: drive family (fuzz / distortion / booster) ---
    { key: 'fuzzface', menuName: 'Fuzz Face', category: 'drive', ttl: 'gx_fuzzface.lv2/gx_fuzzface.ttl' },
    { key: 'fuzzfacefm', menuName: 'Fuzz Face FM', category: 'drive', ttl: 'gx_fuzzfacefm.lv2/gx_fuzzfacefm.ttl' },
    { key: 'fumaster', menuName: 'Fuzz Master', category: 'drive', ttl: 'gx_fumaster.lv2/gx_fumaster.ttl' },
    { key: 'hornet', menuName: 'Hornet', category: 'drive', ttl: 'gx_hornet.lv2/gx_hornet.ttl' },
    { key: 'muff', menuName: 'Muff', category: 'drive', ttl: 'gx_muff.lv2/gx_muff.ttl' },
    { key: 'cstb', menuName: 'Tone Bender', category: 'drive', ttl: 'gx_cstb.lv2/gx_cstb.ttl' },
    { key: 'aclipper', menuName: 'Rat', category: 'drive', ttl: 'gx_aclipper.lv2/gx_aclipper.ttl' },
    { key: 'mxrdist', menuName: 'MXR Distortion', category: 'drive', ttl: 'gx_mxrdist.lv2/gx_mxrdist.ttl' },
    { key: 'rangem', menuName: 'Range Master', category: 'drive', ttl: 'gx_rangem.lv2/gx_rangem.ttl' },
    { key: 'mole', menuName: 'Mole', category: 'drive', ttl: 'gx_mole.lv2/gx_mole.ttl' },
    { key: 'hfb', menuName: 'HF Brightener', category: 'drive', ttl: 'gx_hfb.lv2/gx_hfb.ttl' },
    { key: 'hogsfoot', menuName: 'Hogs Foot', category: 'drive', ttl: 'gx_hogsfoot.lv2/gx_hogsfoot.ttl' },
    { key: 'softclip', menuName: 'Softclip', category: 'drive', orphan: 'softclip.cc' },
    { key: 'bassbooster', menuName: 'Bass Booster', category: 'drive', orphan: 'bassbooster.cc' },
    { key: 'highbooster', menuName: 'High Booster', category: 'drive', orphan: 'highbooster.cc' },

    // --- Phase 1-a: dynamics family ---
    { key: 'expander', menuName: 'Expander', category: 'dynamics', ttl: 'gx_expander.lv2/gx_expander.ttl' },
    { key: 'susta', menuName: 'Sustainer', category: 'dynamics', ttl: 'gx_susta.lv2/gx_susta.ttl' },

    // --- Phase 1-b: eq family ---
    { key: 'graphiceq', menuName: 'Graphic EQ', category: 'eq', ttl: 'gx_graphiceq.lv2/gx_graphiceq.ttl' },
    { key: 'selecteq', menuName: 'Scaleable EQ', category: 'eq', orphan: 'selecteq.cc' },
    { key: 'tonecontroll', menuName: '3 Band EQ', category: 'eq', orphan: 'tonecontroll.cc', stereo: true },
    { key: 'moog', menuName: 'Moog Filter', category: 'eq', orphan: 'moog.cc', stereo: true },
    { key: 'low_high_pass', menuName: 'Low/High Filter', category: 'eq', orphan: 'low_high_pass.cc' },
    { key: 'noise_shaper', menuName: 'Noise Shaper', category: 'eq', orphan: 'noise_shaper.cc' },

    // --- Phase 1-c: wah family ---
    { key: 'wahmodel', menuName: 'Wah Model', category: 'wah', ttl: 'gx_colwah.lv2/gx_colwah.ttl' },
    { key: 'crybaby', menuName: 'Crybaby', category: 'wah', orphan: 'crybaby.cc' },
    { key: 'autowah', menuName: 'Auto Wah', category: 'wah', params: [] },
    { key: 'dunwah', menuName: 'Classic Wah', category: 'wah', params: [{ port: 3, symbol: 'WAH', name: 'Wah', default: 0, min: 0, max: 1, step: 0.01 }] },

    // --- Phase 1-d: modulation family ---
    // vibe: bundle-LOCAL dsp (gx_vibe.lv2/vibe.cc) — the ttl port space is
    // the wrapper enum from gx_vibe.h, and the class ships plugin_stereo()
    // + plugin_mono(); we instantiate the STEREO one (ttl declares in/in1).
    { key: 'vibe', menuName: 'Vibe', category: 'modulation', ttl: 'gx_vibe.lv2/gx_vibe.ttl' },
    { key: 'tubetremelo', menuName: 'Tube Tremolo', category: 'modulation', ttl: 'gxtubetremelo.lv2/gxtubetremelo.ttl' },
    { key: 'tubevibrato', menuName: 'Tube Vibrato', category: 'modulation', ttl: 'gxtubevibrato.lv2/gxtubevibrato.ttl' },
    { key: 'switched_tremolo', menuName: 'Switched Tremolo', category: 'modulation', ttl: 'gx_switched_tremolo.lv2/gx_switched_tremolo.ttl' },
    // classic faust orphans; phaser/flanger are natively 2-in/2-out classes
    // (stereo_audio only) — `stereo: true` overrides the orphan default so
    // the host runs their stereo path instead of a dead dual-mono.
    { key: 'phaser_st', menuName: 'Classic Phaser', category: 'modulation', orphan: 'phaser.cc', stereo: true },
    { key: 'flanger_st', menuName: 'Classic Flanger', category: 'modulation', orphan: 'flanger.cc', stereo: true },
    { key: 'chorus_mono', menuName: 'Chorus Mono', category: 'modulation', orphan: 'chorus_mono.cc' },

    // --- Phase 1-e: time/delay family ---
    // duck/digital delays are bundle-LOCAL dsp (their .lv2 dirs ship the
    // .cc); ttl port space = the wrapper enums from the gx_*.h headers.
    // digital_delay(.st): SYNC + HOSTBPM are wrapper-level LV2 host-tempo
    // ports the faust class ignores — skipPorts drops them from the param
    // mirror (their port indexes stay holes; the DSP connect() default-case
    // discards them anyway).
    { key: 'duck_delay', menuName: 'Duck Delay', category: 'delay', ttl: 'gx_duck_delay.lv2/gx_duck_delay.ttl' },
    { key: 'duck_delay_st', menuName: 'Duck Delay Stereo', category: 'delay', ttl: 'gx_duck_delay_st.lv2/gx_duck_delay_st.ttl' },
    { key: 'digital_delay', menuName: 'Digital Delay', category: 'delay', ttl: 'gx_digital_delay.lv2/gx_digital_delay.ttl', skipPorts: ['SYNC', 'HOSTBPM'] },
    { key: 'digital_delay_st', menuName: 'Digital Delay Stereo', category: 'delay', ttl: 'gx_digital_delay_st.lv2/gx_digital_delay_st.ttl', skipPorts: ['SYNC', 'HOSTBPM'] },
    // tape sims (12au7 tables) + the Copicat tape-echo circuit sim + the
    // 12ax7 tube delay — standard faust-generated classes
    { key: 'gxtape', menuName: 'Tape', category: 'delay', ttl: 'gxtape.lv2/gxtape.ttl' },
    { key: 'gxtape_st', menuName: 'Tape Stereo', category: 'delay', ttl: 'gxtape_st.lv2/gxtape_st.ttl' },
    { key: 'gxechocat', menuName: 'Echo Cat', category: 'delay', ttl: 'gxechocat.lv2/gxechocat.ttl' },
    { key: 'gxtubedelay', menuName: 'Tube Delay', category: 'delay', ttl: 'gxtubedelay.lv2/gxtubedelay.ttl' },
    // circuit-modelled TS-9: menu category DRIVE (Tubescreamer-style
    // overdrive) even though its TU is the time family
    { key: 'ts9', menuName: 'TS-9', category: 'drive', ttl: 'gxts9.lv2/gxts9.ttl' },
    // Boss OC-2 style octave divider — SPECIAL (no pitch category in the
    // frozen vocabulary; the plan parks octavers there)
    { key: 'oc_2', menuName: 'OC-2 Octave', category: 'special', ttl: 'gx_oc_2.lv2/gx_oc_2.ttl' },
    // classic MONO faust orphans — named "Classic ..." to stay distinct
    // from v1's STEREO delay id 8 (stereodelay.cc) / echo id 9
    // (stereoecho.cc)
    { key: 'classic_delay', menuName: 'Classic Delay', category: 'delay', orphan: 'delay.cc' },
    { key: 'classic_echo', menuName: 'Classic Echo', category: 'delay', orphan: 'echo.cc' },

    // --- Phase 1-f: reverb family ---
    // zita_rev1: named "Zita Reverb" to stay distinct from v1's id 10
    // "Reverb" (stereoverb.cc via the wrapper-only gx_reverb.lv2 bundle).
    // room_simulator + shimmizita are bundle-LOCAL dsp (their .lv2 dirs
    // ship the .cc); ttl port space = the wrapper enums from the gx_*.h
    // headers. freeverb is the classic mono faust orphan.
    { key: 'zita_rev1', menuName: 'Zita Reverb', category: 'reverb', ttl: 'gx_zita_rev1.lv2/gx_zita_rev1.ttl' },
    { key: 'freeverb', menuName: 'Freeverb', category: 'reverb', orphan: 'freeverb.cc' },
    { key: 'room_simulator', menuName: 'Room Simulator', category: 'reverb', ttl: 'gx_room_simulator.lv2/gx_room_simulator.ttl' },
    { key: 'shimmizita', menuName: 'Shimmizita', category: 'reverb', ttl: 'gx_shimmizita.lv2/gx_shimmizita.ttl' },

    // --- Phase 1-g: amp + tonestack family ---
    // ampmodel: HOST-SIDE AGGREGATE over the 19 gxamp model classes
    // (gxamp.cc + gxamp2..18.cc + gxnoamp.cc), model order == the gx_amp.ttl
    // `model` scale points == upstream amp_model[]. The ttl describes the
    // WRAPPER's full chain (head + tonestack + cab); skipPorts drops the
    // tonestack (Middle/Bass/Treble/t_model) and cab (Cabinet/Presence/
    // c_model) stages plus the wrapper-only trim, so the exposed surface is
    // the amp-model params + MODEL only (fx2plan: port ONLY the model
    // classes; cabs are Phase 2 convolver).
    { key: 'ampmodel', menuName: 'Amp Model', category: 'amp', ttl: 'gx_amp.lv2/gx_amp.ttl', skipPorts: ['Middle', 'Bass', 'Treble', 'Cabinet', 'Presence', 't_model', 'c_model', 'trim'] },
    // tonestack: HOST-SIDE AGGREGATE over the 27 STEREO tonestack classes;
    // model 0..26 = upstream tonestack_model[] order == the gx_amp.ttl
    // t_model scale points minus the historical "Off" hole. No standalone
    // gx_tonestack.lv2 bundle exists (the wrapper lives in gx_amp.lv2/
    // gx_tonestack.cc), so the surface is inline: Bass/Middle/Treble ranges
    // from gx_amp.ttl ports 4..6, MODEL at t_model's port 10 — the
    // wrapper-space indexes the aggregate forwards untranslated.
    { key: 'tonestack', menuName: 'Tone Stack', category: 'tonestack', stereo: true, params: [
        { port: 4, symbol: 'Middle', name: 'Middle', default: 0.5, min: 0, max: 1, step: 0.01 },
        { port: 5, symbol: 'Bass', name: 'Bass', default: 0.5, min: 0, max: 1, step: 0.01 },
        { port: 6, symbol: 'Treble', name: 'Treble', default: 0.5, min: 0, max: 1, step: 0.01 },
        { port: 10, symbol: 'Model', name: 'Model', default: 0, min: 0, max: 26, step: 1, integer: true },
    ] },
    // preamps (plain classes — convolver-based redeye/metal/cabinet are
    // Phase 2, gx_preamp.cc references engine headers): studiopre ships the
    // STEREO variant (separate native-stereo class; the mono .cc shares the
    // guard-less alembic_* tables so only one can ride the TU). alembic has
    // no tables; w20 embeds its own tiltdrivepro-derived w20 tables.
    { key: 'studiopre', menuName: 'Studio Pre', category: 'amp', ttl: 'gx_studiopre_st.lv2/gx_studiopre_st.ttl' },
    { key: 'alembic', menuName: 'Alembic Pre', category: 'amp', ttl: 'gx_alembic.lv2/gx_alembic.ttl' },
    { key: 'w20', menuName: 'W20 Pre', category: 'amp', ttl: 'gx_w20.lv2/gx_w20.ttl' },

    // --- Phase 1-h: multiband family ---
    // The gx_mb* bundles wrap the faust mbc/mbdel/mbd/mbe classes; ttl port
    // space = the wrapper enums from the gx_mb*.h headers. Meter OUTPUT
    // ports (V*) land in out_ports; the factories park them on TU-local
    // scratch (unconditional #define-deref in compute — graphiceq
    // precedent). barkgraphiceq's DSP is bundle-LOCAL (barkgraphiceq.cc +
    // orfanidis_eq.h + bark_freq_grid.h) like vibe/duck_delay.
    { key: 'mbcompressor', menuName: 'MB Compressor', category: 'multiband', ttl: 'gx_mbcompressor.lv2/gx_mbcompressor.ttl' },
    { key: 'mbdelay', menuName: 'MB Delay', category: 'multiband', ttl: 'gx_mbdelay.lv2/gx_mbdelay.ttl' },
    { key: 'mbdistortion', menuName: 'MB Distortion', category: 'multiband', ttl: 'gx_mbdistortion.lv2/gx_mbdistortion.ttl' },
    { key: 'mbecho', menuName: 'MB Echo', category: 'multiband', ttl: 'gx_mbecho.lv2/gx_mbecho.ttl' },
    { key: 'barkgraphiceq', menuName: 'Bark Graphic EQ', category: 'multiband', ttl: 'gx_barkgraphiceq.lv2/gx_barkgraphiceq.ttl' },

    // gx_bmp ("GxBigMuffPi") — found by the Phase 1-h closing enumeration
    // audit: a real, working, plain-class insert the plan's own lists never
    // enumerate (distinct from gx_muff: the Big Muff PI circuit with the
    // extra SUSTAIN stage; NOT the gx_fuzz composite's bmfp part). Menu
    // category drive.
    { key: 'bigmuffpi', menuName: 'Big Muff Pi', category: 'drive', ttl: 'gx_bmp.lv2/gx_bmp.ttl' },

    // --- Phase 1-h: utility family ---
    // Orphan classes; balance/gx_outputlevel are natively 2-in/2-out
    // (stereo_audio only) so `stereo: true` overrides the orphan default;
    // gx_ampout is mono (dual-mono host).
    { key: 'balance', menuName: 'Balance', category: 'utility', orphan: 'balance.cc', stereo: true },
    { key: 'outputlevel', menuName: 'Output Level', category: 'utility', orphan: 'gx_outputlevel.cc', stereo: true },
    { key: 'ampout', menuName: 'Amp Out', category: 'utility', orphan: 'gx_ampout.cc' },
    { key: 'ampmodul', menuName: 'Postamp', category: 'utility', orphan: 'gx_ampmodul.cc', stereo: true },

    // --- Phase 2-a: convolution family (convolver core) ---
    // cabinet: gx_cabinet.lv2 over a partitioned convolver. The ttl's 4
    // control inputs parse directly (CLevel/CBass/CTreble/c_model); BYPASS
    // (enabled designation), SCHEDULE (notOnGUI output) and the atom ports
    // are filtered by the standard policy. One audio input → mono →
    // dual-mono host. c_model 18 "Off" = dry passthrough in the wrapper.
    // MEMORY-CLASS (cab IR buffers).
    { key: 'cabinet', menuName: 'Cabinet', category: 'amp', ttl: 'gx_cabinet.lv2/gx_cabinet.ttl' },

    // --- Phase 2-b: redeye + metal convolver family ---
    // redeye: HOST-SIDE AGGREGATE folding gx_redeye.lv2's THREE LV2
    // descriptors (#chump/#bigchump/#vibrochump over DSP/gx_redeye.h's
    // amp_model[]) into ONE menu entry with a MODEL param 0..2; each model
    // also carries its FIXED speaker IR (chump → 1x8, bigchump/vibrochump →
    // 2x12) on the convolver. The ttl's port space is per-descriptor with
    // differing notOnGUI marks (chump hides the vibe knobs, vibrochump
    // shows them) and no MODEL port at all (upstream selects the model by
    // descriptor URI) — so the surface is INLINE: the union of the three
    // descriptors' control ports at the shared gxredeye.h wrapper indexes
    // (GAIN..SINEWAVE = 0..7, AMP_OUTPUT/AMP_INPUT = 8/9) + MODEL at port
    // 10 (tonestack precedent). Defaults follow descriptor 0 (chump):
    // Volume 0.5 (bigchump/vibrochump ship 0.3), Intensity 0 (vibrochump
    // 3.0). MEMORY-CLASS (speaker IRs).
    { key: 'redeye', menuName: 'Redeye', category: 'amp', params: [
        { port: 0, symbol: 'Gain', name: 'Gain', default: 0.5, min: 0, max: 1, step: 0.01 },
        { port: 1, symbol: 'Tone', name: 'Tone', default: 0.5, min: 0, max: 1, step: 0.01 },
        { port: 2, symbol: 'Volume', name: 'Volume', default: 0.5, min: 0, max: 1, step: 0.01 },
        { port: 3, symbol: 'Feedback', name: 'Feedback', default: 0, min: 0, max: 1, step: 1, integer: true },
        { port: 4, symbol: 'Vibe', name: 'Vibe', default: 0, min: 0, max: 1, step: 1, integer: true },
        { port: 5, symbol: 'Speed', name: 'Speed', default: 5, min: 0.1, max: 10, step: 0.1 },
        { port: 6, symbol: 'Intensity', name: 'Intensity', default: 0, min: 0, max: 10, step: 0.1 },
        { port: 7, symbol: 'Sinewave', name: 'Sinewave', default: 0, min: 0, max: 1, step: 1, integer: true },
        { port: 10, symbol: 'Model', name: 'Model', default: 0, min: 0, max: 2, step: 1, integer: true },
    ] },
    // metal amp / metal head: the gxmetal_*.lv2 wrappers over the faust
    // gxmetal_{amp,head}.cc preamps + a FIXED cab_data_4x12 convolution
    // stage. ttl parses directly (TONE/DRIVE/PREGAIN/GAIN1); HIGHGAIN
    // (notOnGUI) is filtered. The bundles' gx_metalamp.cc is a DEAD file
    // (not built upstream) — the live wrapper is gxmetal_amp.cpp. The two
    // differ in preamp class + DRIVE range (amp 1..20, head 0..1).
    // MEMORY-CLASS (cab_data IRs).
    { key: 'metalamp', menuName: 'Metal Amp', category: 'amp', ttl: 'gxmetal_amp.lv2/gxmetal_amp.ttl' },
    { key: 'metalhead', menuName: 'Metal Head', category: 'amp', ttl: 'gxmetal_head.lv2/gxmetal_head.ttl' },

    // --- Phase 2-c: detune (phase vocoder) ---
    // detune: gx_detune.lv2's bundle-LOCAL smbPitchShift phase vocoder
    // (Bernsee/guitarix). The ttl parses directly (DETUNE/OCTAVE/
    // COMPENSATE/LATENCY/WET/DRY/LOW/MIDDLELOW/MIDDLETREBLE/TREBLE); BYPASS
    // (enabled designation) is filtered, and the latency OUTPUT port lands
    // in out_ports (never a param). One audio input → mono → dual-mono
    // host. Each smbPitchShift object embeds fixed MAX_FRAME_LENGTH(8096)
    // frame arrays (~420 KB) — ~840 KB per slot dual-mono; SUB-megabyte,
    // so NOT memory-class flagged.
    { key: 'detune', menuName: 'Detune', category: 'special', ttl: 'gx_detune.lv2/gx_detune.ttl' },

    // --- Phase 2-d: tuner (inline pitch tracker) ---
    // tuner: gxtuner.lv2's PitchTracker (NSDF detector) driven inline.
    // INLINE surface because the ttl cannot be trusted: its tail port order
    // disagrees with gxtuner.h's PortIndex (VERIFY at 15 vs 18), and most
    // of its control inputs are MIDI machinery (CHANNEL/ONMIDI/PITCHBEND/
    // SINGLENOTE/BPM/VELOCITY/VERIFY/GATE/SYNTHFREQ/GAIN — dropped) or
    // GUI-only state the DSP wrapper never even connects (TUNER_MODE/
    // TEMPERAMENT/MAXL/RESET). Shipped: REFFREQ (display reference — the
    // engine never reads it, the UI's note math shifts A4 by it) +
    // THRESHOLD (tracker gate, dB). FREQ (ttl port 0) is a control OUTPUT
    // (out_ports).
    { key: 'tuner', menuName: 'Tuner', category: 'special', params: [
        { port: 1, symbol: 'REFFREQ', name: 'Reference Pitch', default: 440, min: 427, max: 453, step: 0.1 },
        { port: 4, symbol: 'THRESHOLD', name: 'Threshold', default: -50, min: -60, max: 4, step: 0.5 },
    ], out_ports: [
        { port: 0, symbol: 'FREQ', name: 'Frequency', min: 0, max: 1000 },
    ] },

    // --- Phase 3: live looper ---
    // livelooper: gx_livelooper.lv2's bundle-LOCAL, hand-tweaked faust
    // class (livelooper.cc — NOT the gx_head/engine LiveLooper). The ttl
    // parses directly once keepPorts rescues the pprop:trigger momentary
    // buttons the DSP actually consumes as plain toggles (reset1..4 clear
    // the tapes, rback1..4 rewind to the clip end): 39 control inputs + 8
    // control OUTPUT meter ports (bar1..4 remaining record time, playh1..4
    // play-head per-mille). One ttl audio input → mono → dual-mono host.
    // MEMORY: TAPESIZE 4194304 floats = 16 MiB x 4 tapes = 64 MiB per DSP
    // instance, lazily new'd on activate — MEMORY-CLASS flagged (its own
    // stricter cap comes with the looper wave).
    { key: 'livelooper', menuName: 'Live Looper', category: 'special', ttl: 'gx_livelooper.lv2/gx_livelooper.ttl',
      keepPorts: ['reset1', 'reset2', 'reset3', 'reset4', 'rback1', 'rback2', 'rback3', 'rback4'] },

];

const FX_COUNT = MANIFEST.length;

// ---------------------------------------------------------------------------
// ttl parsing (bundle .ttl — lv2:port blocks)
// ---------------------------------------------------------------------------

/** Parse the `lv2:port` list of a guitarix bundle ttl into typed ports. */
function parseTtlPorts(rel) {
    const text = readFileSync(join(ROOT, GX_LV2, rel), 'utf8');
    const m = text.match(/lv2:port\s*\[([\s\S]*?)\]\s*\./);
    if (!m) throw new Error(`${rel}: no lv2:port block found`);
    return m[1].split(/\]\s*,\s*\[/).map((raw) => {
        const port = { types: [], props: [], designation: [] };
        for (const stmt of raw.split(';')) {
            const s = stmt.trim().replace(/^-\s*/, '');
            if (!s) continue;
            const mm = s.match(/^([\w:]+)\s+([\s\S]*)$/);
            if (!mm) continue;
            const pred = mm[1];
            const objs = mm[2].split(',').map((o) => o.trim().replace(/;$/, '')).filter(Boolean);
            for (const obj of objs) {
                if (pred === 'a') port.types.push(obj);
                else if (pred.endsWith('index')) port.index = Number(obj);
                else if (pred.endsWith('symbol')) port.symbol = obj.replace(/^"|"$/g, '');
                else if (pred.endsWith('name')) port.name = obj.replace(/^"|"$/g, '');
                else if (pred.endsWith('default')) port.def = Number(obj);
                else if (pred.endsWith('minimum')) port.min = Number(obj);
                else if (pred.endsWith('maximum')) port.max = Number(obj);
                else if (pred.endsWith('portProperty')) port.props.push(obj);
                else if (pred.endsWith('designation')) port.designation.push(obj);
            }
        }
        return port;
    });
}

function ttlEffect(entry) {
    const ports = parseTtlPorts(entry.ttl);
    const problems = [];
    const isA = (p, suffix) => p.types.some((t) => t.endsWith(suffix));
    for (const p of ports) {
        if (!Number.isInteger(p.index)) problems.push(`port ${p.symbol ?? '?'}: missing lv2:index`);
        if (!p.symbol) problems.push(`port index ${p.index}: missing symbol`);
    }
    const audioIns = ports.filter((p) => isA(p, 'AudioPort') && isA(p, 'InputPort'));
    const skip = new Set(entry.skipPorts || []);
    // keepPorts: symbols the trigger/notOnGUI screen must NOT drop (the
    // DSP reads them as plain controls even though the ttl marks them as
    // momentary buttons). Livelooper precedent: reset/rback — upstream's
    // UI fires them as triggers, the class consumes them as toggles (and
    // self-clears reset once the tape is empty).
    const keep = new Set(entry.keepPorts || []);
    const paramPorts = ports.filter((p) =>
        isA(p, 'ControlPort') && isA(p, 'InputPort')
        && !skip.has(p.symbol)
        && !p.designation.some((d) => d.endsWith('enabled'))
        && (keep.has(p.symbol)
            || !p.props.some((x) => x.endsWith('trigger') || x.endsWith('notOnGUI'))));
    paramPorts.sort((a, b) => a.index - b.index);
    // Control OUTPUT ports (meters): parsed for completeness (spec
    // out_ports) but never emitted — CakeMix's engine has no out-port
    // surface yet.
    const outPorts = ports.filter((p) => isA(p, 'ControlPort') && isA(p, 'OutputPort'));
    outPorts.sort((a, b) => a.index - b.index);
    if (paramPorts.length === 0) problems.push('no param ports after filtering');
    if (problems.length > 0) throw new Error(`${entry.ttl}: ${problems.join('; ')}`);

    return {
        stereo: audioIns.length >= 2,
        out_ports: outPorts.map((p) => ({
            port: p.index,
            symbol: p.symbol,
            name: humanLabel(p.name || p.symbol, p.symbol),
            min: p.min,
            max: p.max,
        })),
        params: paramPorts.map((p) => {
            const toggled = p.props.some((x) => x.endsWith('toggled'));
            const integer = p.props.some((x) => x.endsWith('integer'));
            return {
                port: p.index,
                symbol: p.symbol,
                name: humanLabel(p.name || p.symbol, p.symbol),
                default: p.def,
                min: p.min,
                max: p.max,
                step: stepFor(p.min, p.max, toggled, integer),
                toggled,
                integer,
            };
        }),
    };
}

// ---------------------------------------------------------------------------
// orphan parsing (faust-generated .cc — connect_ports comments + enum)
// ---------------------------------------------------------------------------

/** Parse an orphan faust class: param metadata from the connect_ports
 * `// , default, min, max, step` comments, port index = position in the
 * trailing `typedef enum {...} PortIndex` comment. */
function orphanEffect(entry) {
    const text = readFileSync(join(ROOT, GX_FAUST, entry.orphan), 'utf8');
    const em = text.match(/\/\*\s*typedef\s+enum\s*\{([^}]*)\}\s*PortIndex\s*;?\s*\*\//);
    if (!em) throw new Error(`${entry.orphan}: no trailing PortIndex enum comment`);
    const symbols = em[1].split(',').map((s) => s.trim()).filter(Boolean);
    const params = [];
    // `f`-suffix tolerance: some .cc files carry C float literals in the
    // comments (crybaby: `// , 0.1f, 0.0f, 1.0f, 0.01f`), others plain
    // decimals (softclip: `// , 0.0, 0.0, 1.99, 0.01`).
    const caseRe = /case\s+(\w+)\s*:\s*\n\s*\w+\s*=\s*\(float\*\)data;\s*\/\/\s*,\s*([-\d.eE+]+)f?\s*,\s*([-\d.eE+]+)f?\s*,\s*([-\d.eE+]+)f?\s*,\s*([-\d.eE+]+)f?/g;
    let cm;
    while ((cm = caseRe.exec(text)) !== null) {
        const port = symbols.indexOf(cm[1]);
        if (port < 0) throw new Error(`${entry.orphan}: case ${cm[1]} not in PortIndex enum`);
        params.push({
            port,
            symbol: cm[1],
            name: humanLabel(cm[1], cm[1]),
            default: Number(cm[2]),
            min: Number(cm[3]),
            max: Number(cm[4]),
            step: Number(cm[5]),
            toggled: false,
            integer: false,
        });
    }
    if (params.length === 0) throw new Error(`${entry.orphan}: no connect_ports param comments`);
    params.sort((a, b) => a.port - b.port);
    return { stereo: false, out_ports: [], params };
}

// ---------------------------------------------------------------------------
// inline params (hand-authored manifest rows — DSP with no parseable ttl)
// ---------------------------------------------------------------------------

/** Build an effect from hand-authored param rows. Validation (port bounds,
 * ascending order, min<max, default in range, step>0) is the shared
 * invariant loop in normalize(); inline is the ONLY source kind allowed
 * zero params (an envelope-driven effect with no controls at all).
 * `out_ports` (optional, same shape as the ttl out_ports) declares the
 * effect's control OUTPUT ports. */
function inlineEffect(entry) {
    if (!Array.isArray(entry.params)) throw new Error('inline entry needs a params array');
    return {
        stereo: false,
        out_ports: (entry.out_ports || []).map((o) => ({
            port: o.port,
            symbol: o.symbol,
            name: o.name || o.symbol,
            min: o.min,
            max: o.max,
        })),
        params: entry.params.map((p) => ({
            port: p.port,
            symbol: p.symbol,
            name: p.name || p.symbol,
            default: p.default,
            min: p.min,
            max: p.max,
            step: p.step,
            toggled: false,
            integer: !!p.integer,
        })),
    };
}

// ---------------------------------------------------------------------------
// label humanization + step derivation
// ---------------------------------------------------------------------------

// Symbols/names that get hand-v1-style names instead of plain Title Case.
const LABEL_OVERRIDES = {
    WET_DRY: 'Dry/Wet',
    DRY_WET: 'Dry/Wet',
    INPUT: 'Input',
    AUDIO_IN: 'Input',
    // --- Phase 1-b eq family (dsp2cc uppercases + mangles band suffixes;
    // names follow the .dsp tooltips: Q per band, freq Hz, gain dB) ---
    // moog
    FR: 'Frequency',
    // selecteq — 10 bands × (quality, freq, gain)
    QS31_25: 'Q 31.25', QS62_5: 'Q 62.5', QS125: 'Q 125', QS250: 'Q 250', QS500: 'Q 500',
    QS1K: 'Q 1k', QS2K: 'Q 2k', QS4K: 'Q 4k', QS8K: 'Q 8k', QS16K: 'Q 16k',
    FREQ31_25: 'Freq 31.25', FREQ62_5: 'Freq 62.5', FREQ125: 'Freq 125', FREQ250: 'Freq 250',
    FREQ500: 'Freq 500', FREQ1K: 'Freq 1k', FREQ2K: 'Freq 2k', FREQ4K: 'Freq 4k',
    FREQ8K: 'Freq 8k', FREQ16K: 'Freq 16k',
    FS31_25: 'Gain 31.25', FS62_5: 'Gain 62.5', FS125: 'Gain 125', FS250: 'Gain 250',
    FS500: 'Gain 500', FS1K: 'Gain 1k', FS2K: 'Gain 2k', FS4K: 'Gain 4k',
    FS8K: 'Gain 8k', FS16K: 'Gain 16k',
    // low_high_pass — two stages in one class: the speaker band-pass
    // (LOWFREQ/HIGHFREQ/ONOFF) and the low/high-pass (LOW_FREQ/HIGH_FREQ/ON_OFF)
    LOWFREQ: 'BP Low Freq',
    HIGHFREQ: 'BP High Freq',
    ONOFF: 'BP On/Off',
    LOW_FREQ: 'LP Freq',
    HIGH_FREQ: 'HP Freq',
    ON_OFF: 'LP/HP On/Off',
    // --- Phase 1-d modulation family (ttl WETDRY/FB + dsp2cc-mangled
    // multi-word symbols; names follow the .dsp tooltips) ---
    // vibe (gx_vibe.ttl)
    WETDRY: 'Dry/Wet',
    FB: 'Feedback',
    DF: 'L/R Phase',        // lfo phase offset between the two channels (turns)
    // gxtubetremelo / gxtubevibrato (ttl symbol is lowercase "sinewave")
    sinewave: 'Sine Wave',
    SineWave: 'Sine Wave',
    // switched_tremolo
    FREQ0: 'Freq 0',
    FREQ1: 'Freq 1',
    FREQ2: 'Freq 2',
    FREQ3: 'Freq 3',
    SWITCHFREQ: 'Switch Freq',
    // classic phaser / flanger orphans
    MAXNOTCH1FREQ: 'Max Notch Freq',
    MINNOTCH1FREQ: 'Min Notch Freq',
    NOTCHWIDTH: 'Notch Width',
    NOTCHFREQ: 'Notch Freq',
    VIBRATOMODE: 'Vibrato Mode',
    FEEDBACKGAIN: 'Feedback Gain',
    LFOFREQ: 'LFO Freq',
    DELAYOFFSET: 'Delay Offset',
    // --- Phase 1-e time/delay family (keys chosen to NOT collide with any
    // symbol/name of effects 0..44 — regeneration keeps those byte-identical;
    // e.g. there is deliberately NO 'DELAY' override: flanger_st id 43
    // already carries that symbol) ---
    // duck_delay / duck_delay_st (ttl names are the upper-case symbols)
    RELESE: 'Release',      // upstream's misspelling of RELEASE (mono variant)
    PINGPONG: 'Ping-Pong',
    // digital_delay / digital_delay_st
    BPM: 'BPM',
    bpm: 'BPM',             // gxechocat ttl symbol is lower-case
    HOWPASS: 'Lowpass',     // upstream's misspelling — it is the low cut
    // gxtape / gxtape_st (ttl symbols are lower-case camel)
    wowdepth: 'Wow Depth',
    wowfreq: 'Wow Freq',
    flutdepth: 'Flutter Depth',
    flutfreq: 'Flutter Freq',
    hiss: 'Tape Hiss',
    type: 'Tape Type',
    // gxechocat
    head1: 'Head 1',
    head2: 'Head 2',
    head3: 'Head 3',
    // gx_oc_2
    OCTAVE1: 'Octave 1',
    OCTAVE2: 'Octave 2',
    // classic echo orphan (PERCENT = wet share)
    PERCENT: 'Wet %',
    // --- Phase 1-f reverb family (keys chosen to NOT collide with any
    // symbol/name of effects 0..56 — regeneration keeps those
    // byte-identical; e.g. there are deliberately NO 'LEVEL'/'EFFECT'/
    // 'DEPTH'/'MODE'/'SPEED' overrides: gxechocat/gxtubedelay carry LEVEL,
    // duck_delay_st EFFECT, vibe DEPTH, digital_delay MODE, gxtape speed) ---
    // gx_zita_rev1 (ttl symbols; the ttl spells the level port "level")
    EQ1_FREQ: 'EQ1 Freq',
    EQ1_LEVEL: 'EQ1 Level',
    EQ2_FREQ: 'EQ2 Freq',
    EQ2_LEVEL: 'EQ2 Level',
    IN_DELAY: 'Predelay',
    LOW_RT60: 'Low RT60',
    MID_RT60: 'Mid RT60',
    LF_X: 'LF X',
    HF_DAMPING: 'HF Damping',
    DRY_WET_MIX: 'Dry/Wet Mix',
    // gx_room_simulator (RT is the feedback decay multiplier in the dsp)
    PREDELAYMS: 'Predelay Ms',
    RT: 'Decay',
    ROOMSIZE: 'Room Size',
    DRYWET: 'Dry/Wet',
    // gx_shimmizita (t60ds/t60m = decay at freq 0 / midrange per the .inc
    // docs; CONTROL = envelope-follower-to-pitch-shifter influence)
    PSDRYWET: 'Pitch Dry/Wet',
    SHIFT: 'Pitch Shift',
    T60DS: 'T60 Low',
    T60M: 'T60 Mid',
    CONTROL: 'Env Control',
    // freeverb orphan
    DAMP: 'Damping',
    // --- Phase 1-g amp + tonestack family (keys chosen to NOT collide with
    // any symbol/name of effects 0..60 — regeneration keeps those
    // byte-identical; 'model' (lowercase, gx_amp.ttl) needs no override,
    // titleCase yields "Model") ---
    // gx_amp.ttl amp-model ports (EnhancedUI camelCase symbols)
    MasterGain: 'Master Gain',
    PreGain: 'Pre Gain',
    HIGHGAIN: 'High Gain',
    // --- Phase 2-b metal convolver family (keys chosen to NOT collide with
    // any symbol/name of effects 0..76 — GAIN1/TONE/DRIVE deliberately left
    // alone: GAIN1 already rides ampmodel id 61 as "Gain1") ---
    PREGAIN: 'Pre Gain',    // gxmetal_*.ttl (cf. ampmodel's camel PreGain)
    // gx_studiopre_st.ttl port 12 carries upstream's name typo "master_L"
    // on the R-channel master — key the symbol, not the name.
    master_r: 'Master R',
    // --- Phase 2-c detune (keys chosen to NOT collide with any symbol/name
    // of effects 0..79 — regeneration keeps those byte-identical; the other
    // detune symbols titleCase cleanly: WET→Wet, DRY→Dry, LOW→Low,
    // TREBLE→Treble, MIDDLELOW/MIDDLETREBLE are dsp2cc word-mashes) ---
    MIDDLELOW: 'Middle Low',    // gx_detune.ttl
    MIDDLETREBLE: 'Mid Treble', // gx_detune.ttl
    // --- Phase 1-h multiband + utility family (keys chosen to NOT collide
    // with any symbol/name of effects 0..65 — regeneration keeps those
    // byte-identical; deliberately NO G1..G24 overrides: graphiceq id 28
    // pins "G1".."G11" band labels and bark stays consistent with it;
    // WET_DRY/HIGHGAIN/LEVEL reuse existing overrides/titleCase) ---
    // gx_mbcompressor (digit-suffixed ttl symbols titleCase to "Mode1" etc.)
    MODE1: 'Mode 1', MODE2: 'Mode 2', MODE3: 'Mode 3', MODE4: 'Mode 4', MODE5: 'Mode 5',
    MAKEUP1: 'Makeup 1', MAKEUP2: 'Makeup 2', MAKEUP3: 'Makeup 3', MAKEUP4: 'Makeup 4', MAKEUP5: 'Makeup 5',
    MAKEUPTHRESHOLD1: 'Makeup Thresh 1', MAKEUPTHRESHOLD2: 'Makeup Thresh 2',
    MAKEUPTHRESHOLD3: 'Makeup Thresh 3', MAKEUPTHRESHOLD4: 'Makeup Thresh 4',
    MAKEUPTHRESHOLD5: 'Makeup Thresh 5',
    RATIO1: 'Ratio 1', RATIO2: 'Ratio 2', RATIO3: 'Ratio 3', RATIO4: 'Ratio 4', RATIO5: 'Ratio 5',
    ATTACK1: 'Attack 1', ATTACK2: 'Attack 2', ATTACK3: 'Attack 3', ATTACK4: 'Attack 4', ATTACK5: 'Attack 5',
    RELEASE1: 'Release 1', RELEASE2: 'Release 2', RELEASE3: 'Release 3', RELEASE4: 'Release 4', RELEASE5: 'Release 5',
    // gx_mbdelay
    DELAY1: 'Delay 1', DELAY2: 'Delay 2', DELAY3: 'Delay 3', DELAY4: 'Delay 4', DELAY5: 'Delay 5',
    FEEDBACK1: 'Feedback 1', FEEDBACK2: 'Feedback 2', FEEDBACK3: 'Feedback 3', FEEDBACK4: 'Feedback 4', FEEDBACK5: 'Feedback 5',
    GAIN1: 'Gain 1', GAIN2: 'Gain 2', GAIN3: 'Gain 3', GAIN4: 'Gain 4', GAIN5: 'Gain 5',
    // gx_mbdistortion
    DRIVE1: 'Drive 1', DRIVE2: 'Drive 2', DRIVE3: 'Drive 3', DRIVE4: 'Drive 4', DRIVE5: 'Drive 5',
    OFFSET1: 'Offset 1', OFFSET2: 'Offset 2', OFFSET3: 'Offset 3', OFFSET4: 'Offset 4', OFFSET5: 'Offset 5',
    // gx_mbecho (PERCENTn = per-band wet share — cf. classic_echo 'Wet %')
    PERCENT1: 'Wet 1', PERCENT2: 'Wet 2', PERCENT3: 'Wet 3', PERCENT4: 'Wet 4', PERCENT5: 'Wet 5',
    TIME1: 'Time 1', TIME2: 'Time 2', TIME3: 'Time 3', TIME4: 'Time 4', TIME5: 'Time 5',
    // shared crossover ports (ttl names are the misleading "LOW SHELF")
    CROSSOVER_B1_B2: 'Crossover B1/B2', CROSSOVER_B2_B3: 'Crossover B2/B3',
    CROSSOVER_B3_B4: 'Crossover B3/B4', CROSSOVER_B4_B5: 'Crossover B4/B5',
    // utility orphans
    OUT_MASTER: 'Level',   // gx_outputlevel
    OUT_AMP: 'Level',      // gx_ampout
    FEEDBAC: 'Dry Feedback', // gx_ampmodul dry-path feedback (fback in the .dsp)
    TUBE1: 'Tube 1',       // gx_ampmodul stage1 preamp tube (dB)
    TUBE2: 'Tube 2',       // gx_ampmodul stage2 tube (dB)
    // --- Phase 3 live looper (keys chosen to NOT collide with any
    // symbol/name of effects 0..81 — regeneration keeps those
    // byte-identical; 'gain' and 'mix' deliberately have NO overrides:
    // flanger id 4 and gxtape/gxtape_st/w20 carry those symbols and
    // titleCase yields "Gain"/"Mix" anyway) ---
    clip1: 'Clip 1', clip2: 'Clip 2', clip3: 'Clip 3', clip4: 'Clip 4',
    clips1: 'Clip Size 1', clips2: 'Clip Size 2', clips3: 'Clip Size 3', clips4: 'Clip Size 4',
    speed1: 'Speed 1', speed2: 'Speed 2', speed3: 'Speed 3', speed4: 'Speed 4',
    level1: 'Level 1', level2: 'Level 2', level3: 'Level 3', level4: 'Level 4',
    play1: 'Play 1', play2: 'Play 2', play3: 'Play 3', play4: 'Play 4',
    rplay1: 'Rev Play 1', rplay2: 'Rev Play 2', rplay3: 'Rev Play 3', rplay4: 'Rev Play 4',
    rec1: 'Rec 1', rec2: 'Rec 2', rec3: 'Rec 3', rec4: 'Rec 4',
    reset1: 'Reset 1', reset2: 'Reset 2', reset3: 'Reset 3', reset4: 'Reset 4',
    rback1: 'Rewind 1', rback2: 'Rewind 2', rback3: 'Rewind 3', rback4: 'Rewind 4',
    synct: 'Sync Tapes',   // one transport for all 4 tapes
};

function titleCase(s) {
    return s.replace(/_/g, ' ').split(/\s+/)
        .map((w) => (w ? w[0].toUpperCase() + w.slice(1).toLowerCase() : w))
        .join(' ');
}

function humanLabel(name, symbol) {
    if (symbol in LABEL_OVERRIDES) return LABEL_OVERRIDES[symbol];
    if (name in LABEL_OVERRIDES) return LABEL_OVERRIDES[name];
    return titleCase(name || symbol);
}

/** Deterministic ttl step (ttl carries no step; display metadata only). */
function stepFor(min, max, toggled, integer) {
    if (toggled || integer) return 1;
    const span = max - min;
    if (span <= 3) return 0.01;
    if (span <= 30) return 0.1;
    if (span <= 300) return 1;
    return 10;
}

// ---------------------------------------------------------------------------
// Spec load + validation (v1 pinning)
// ---------------------------------------------------------------------------

function loadSpec() {
    const problems = [];
    let raw;
    try {
        raw = JSON.parse(readFileSync(join(ROOT, SPEC_JSON), 'utf8'));
    } catch (e) {
        console.error(`ERROR: cannot parse ${SPEC_JSON}: ${e.message}`);
        process.exit(2);
    }
    if (raw.version !== 1 && raw.version !== 2) {
        problems.push(`version must be 1 or 2, got ${JSON.stringify(raw.version)}`);
    }
    if (!Array.isArray(raw.effects)) {
        problems.push('effects must be an array');
        raw.effects = [];
    }
    if (raw.effects.length < V1_KEYS.length) {
        problems.push(`expected at least ${V1_KEYS.length} effects (v1 pin set), got ${raw.effects.length}`);
    }
    // Only the v1 pin set is validated + consumed; entries beyond it (none
    // in CakeMix's trimmed spec) would be regenerated from the manifest
    // sources on every run.
    for (let i = 0; i < V1_KEYS.length && i < raw.effects.length; i++) {
        const e = raw.effects[i];
        const where = `effects[${i}]`;
        if (e.key !== V1_KEYS[i]) problems.push(`${where}.key must be "${V1_KEYS[i]}" (v1 pin order), got ${JSON.stringify(e.key)}`);
        if (e.id !== i) problems.push(`${where}.id must be ${i}, got ${JSON.stringify(e.id)}`);
        if (typeof e.label !== 'string') problems.push(`${where}.label must be a string`);
        if (typeof e.dsp !== 'string') problems.push(`${where}.dsp must be a string`);
        if (typeof e.stereo !== 'boolean') problems.push(`${where}.stereo must be a boolean`);
        if (!Array.isArray(e.params) || e.params.length === 0) { problems.push(`${where}.params must be a non-empty array`); continue; }
        let prevPort = -1;
        e.params.forEach((p, j) => {
            const pw = `${where}.params[${j}]`;
            if (!Number.isInteger(p.port) || p.port < 0 || p.port >= FX_SLOT_PARAMS) problems.push(`${pw}.port must be an integer in 0..${FX_SLOT_PARAMS - 1}`);
            if (p.port <= prevPort) problems.push(`${pw}.port ${p.port} breaks ascending port order`);
            prevPort = p.port;
            if (typeof p.symbol !== 'string' || typeof p.name !== 'string') problems.push(`${pw}.symbol/.name must be strings`);
            for (const f of ['default', 'min', 'max', 'step']) {
                if (typeof p[f] !== 'number' || !Number.isFinite(p[f])) problems.push(`${pw}.${f} must be a finite number`);
            }
            if (p.min >= p.max) problems.push(`${pw}: min ${p.min} must be < max ${p.max}`);
            if (p.default < p.min || p.default > p.max) problems.push(`${pw}: default ${p.default} outside [${p.min}, ${p.max}]`);
            if (p.step <= 0) problems.push(`${pw}.step must be > 0`);
            if (typeof p.toggled !== 'boolean' || typeof p.integer !== 'boolean') problems.push(`${pw}.toggled/.integer must be booleans`);
        });
    }
    return { raw, problems };
}

// Build the normalized effect list: v1 pinned entries + manifest-parsed new
// entries, ids = manifest index.
function normalize(raw, problems) {
    const effects = MANIFEST.map((entry, i) => {
        let fx;
        if (entry.pin) {
            const e = raw.effects[i];
            fx = {
                id: i,
                key: e.key,
                label: e.label,
                dsp: e.dsp,
                stereo: e.stereo,
                category: entry.category,
                out_ports: [],
                params: e.params.map((p) => ({
                    port: p.port,
                    symbol: p.symbol,
                    name: p.name,
                    default: p.default,
                    min: p.min,
                    max: p.max,
                    step: p.step,
                    toggled: p.toggled,
                    integer: p.integer,
                })),
            };
        } else {
            if (!CATEGORY_VOCAB.has(entry.category)) problems.push(`manifest entry "${entry.key}": category "${entry.category}" not in the frozen vocabulary`);
            let parsed;
            try {
                parsed = entry.ttl ? ttlEffect(entry)
                    : entry.orphan ? orphanEffect(entry)
                    : inlineEffect(entry);
            } catch (err) {
                problems.push(`manifest entry "${entry.key}": ${err.message}`);
                parsed = { stereo: false, params: [] };
            }
            fx = {
                id: i,
                key: entry.key,
                label: entry.menuName,
                dsp: entry.key,
                stereo: entry.stereo !== undefined ? entry.stereo : parsed.stereo,
                category: entry.category,
                out_ports: parsed.out_ports,
                params: parsed.params,
            };
        }
        // Shared invariants (both sources). Empty param lists are legal ONLY
        // for hand-authored inline entries (envelope-driven effects).
        if (fx.params.length === 0 && !Array.isArray(entry.params)) problems.push(`effects[${i}] "${fx.key}": no params`);
        if (fx.params.length > FX_SLOT_PARAMS) problems.push(`effects[${i}] "${fx.key}": ${fx.params.length} params > FX_SLOT_PARAMS ${FX_SLOT_PARAMS}`);
        let prevPort = -1;
        fx.params.forEach((p, j) => {
            const pw = `effects[${i}].params[${j}]`;
            if (!Number.isInteger(p.port) || p.port < 0 || p.port >= FX_PORT_MAX) problems.push(`${pw}.port must be in 0..${FX_PORT_MAX - 1}`);
            if (p.port <= prevPort) problems.push(`${pw}.port breaks ascending order`);
            prevPort = p.port;
            for (const f of ['default', 'min', 'max', 'step']) {
                if (typeof p[f] !== 'number' || !Number.isFinite(p[f])) problems.push(`${pw}.${f} must be a finite number`);
            }
            if (p.min >= p.max) problems.push(`${pw}: min ${p.min} must be < max ${p.max}`);
            if (p.default < p.min || p.default > p.max) problems.push(`${pw}: default ${p.default} outside [${p.min}, ${p.max}]`);
            if (p.step <= 0) problems.push(`${pw}.step must be > 0`);
        });
        fx.memory = isMemoryClass(fx);
        return fx;
    });
    return { effects };
}

// ---------------------------------------------------------------------------
// Deterministic number formatting (JS doubles → minimal round-trip decimals)
// ---------------------------------------------------------------------------

function fmtNum(x) {
    if (typeof x !== 'number' || !Number.isFinite(x)) throw new Error(`non-finite constant: ${x}`);
    const s = String(x);
    if (s.includes('e') || s.includes('E')) throw new Error(`constant ${x} formats exponentially; extend the formatter`);
    return s;
}
// C float literal (always carries a decimal point)
const fC = (x) => (Number.isInteger(x) ? `${x}.0f` : `${fmtNum(x)}f`);

// ---------------------------------------------------------------------------
// Output: crates/mixer-wasm/gxfx/gxfx_catalog.h
// ---------------------------------------------------------------------------

/** Aligned initializer row body (`{ ... },`): strings left-aligned,
 * numbers right-aligned (commas ride the cell so negative signs line
 * up). */
function layoutRow(cells, widths) {
    const parts = cells.map((c, i) => (c.right ? c.s.padStart(widths[i]) : c.s.padEnd(widths[i])));
    return `{ ${parts.join(' ')} },`;
}

const columnWidths = (rows) => rows[0].map((_, i) => Math.max(...rows.map((r) => r[i].s.length)));

function genHeader({ effects }) {
    const maxParams = effects.reduce((m, fx) => Math.max(m, fx.params.length), 0);
    if (maxParams > FX_SLOT_PARAMS) {
        throw new Error(`widest effect has ${maxParams} params > FX_SLOT_PARAMS ${FX_SLOT_PARAMS}`);
    }
    if (maxParams > FX_MAX_LIVE_PARAMS) {
        throw new Error(`widest effect has ${maxParams} params > FX_MAX_LIVE_PARAMS ${FX_MAX_LIVE_PARAMS} — the padded row headroom shrank; update FX_MAX_LIVE_PARAMS deliberately`);
    }
    const counts = effects.map((fx) => fx.params.length).join(',');
    const L = [];
    const push = (...lines) => L.push(...lines);

    push(
        '/*',
        ' * crates/mixer-wasm/gxfx/gxfx_catalog.h — guitarix FX catalog for the',
        ' * CakeMix mixer engine (per-strip insert chains).',
        ' *',
        ' * AUTO-GENERATED by build/gen-gxfx-params.mjs (manifest: ttl bundles +',
        ' * orphan .cc classes in vendor/guitarix, pinned submodule; v1 eleven',
        ' * pinned in build/gxfx-param-spec.json) — DO NOT EDIT BY HAND;',
        ' * regenerate with:',
        ' *     node build/gen-gxfx-params.mjs          (write)',
        ' *     node build/gen-gxfx-params.mjs --check  (freshness gate)',
        ' *',
        ' * Layout (modeled on OctOBX wasm/obxd/gxfx_defaults.h, extended to carry',
        ' * the full per-param UI metadata for the JSON surface):',
        ` *   GXFX_EFFECT_COUNT  catalog size (${FX_COUNT}); row index = effect id`,
        ' *                      (manifest order).',
        ` *   GXFX_CHAIN_SLOTS   FX slots per insert chain (${FX_SLOTS}).`,
        ` *   GXFX_MAX_PARAMS    padded per-effect param row width (${FX_SLOT_PARAMS}; the`,
        ` *                      widest effect is ${maxParams} — asserted at generation time).`,
        ' *   GxFxParamDef       per-param: LV2 PortIndex (`port`, the value passed',
        ' *                      to fx_set_param), symbol, display name, and',
        ' *                      default / min / max / step in ENGINE units (ttl',
        ' *                      ranges, NOT 0..1), plus the integer flag.',
        ' *   GxFxEffectDef      per-effect: key, label, menu category, stereo flag',
        ' *                      (1 = native stereo path, 0 = dual-mono), memory',
        ' *                      flag (1 = MB-scale delay/reverb buffers at',
        ' *                      create/first-activate), live param count, param',
        ' *                      table.',
        ' *   GXFX_CATALOG       all effects in id order; param order = port',
        ` *                      order. Param counts ${counts}.`,
        ' *   FX_DEFAULTS[id][p] per-effect defaults, padded to',
        ` *                      ${FX_SLOT_PARAMS} with 0.0f.`,
        ' *   FX_PORTS[id][p]    LV2 PortIndex per param ordinal, zero-padded.',
        ' *',
        ' * DSP AVAILABILITY is not catalog data: ids whose DSP TU has not been',
        ' * ported yet ride NULL factories in gxfx_dsp_eleven.cpp\'s',
        ' * GXFX_FACTORIES table; gxfx_effect_info reports them unavailable and',
        ' * the engine rejects them at set_slot. Each port wave flips factories',
        ' * from null to live without touching this file.',
        ' */',
        '#ifndef GXFX_CATALOG_H',
        '#define GXFX_CATALOG_H',
        '',
        `#define GXFX_EFFECT_COUNT ${FX_COUNT}`,
        `#define GXFX_CHAIN_SLOTS ${FX_SLOTS}`,
        `#define GXFX_MAX_PARAMS ${FX_SLOT_PARAMS}`,
        '',
        'typedef struct GxFxParamDef {',
        '    int port;              /* LV2 PortIndex passed to fx_set_param */',
        '    const char *symbol;    /* LV2 port symbol */',
        '    const char *name;      /* display name */',
        '    float def;             /* default, ENGINE units (ttl range, not 0..1) */',
        '    float min;',
        '    float max;',
        '    float step;',
        '    int is_integer;        /* 1 = integer-stepped */',
        '} GxFxParamDef;',
        '',
        'typedef struct GxFxEffectDef {',
        '    const char *key;',
        '    const char *label;',
        '    const char *category;  /* menu category string */',
        '    int stereo;            /* 1 = native stereo path, 0 = dual-mono */',
        '    int memory;            /* 1 = MB-scale buffers at create/activate */',
        '    int param_count;       /* live params; rest of each row is padding */',
        '    const GxFxParamDef *params;',
        '} GxFxEffectDef;',
        '',
        '/* Effect-level info block filled by gxfx_effect_info() (the C ABI in',
        ' * gxfx_dsp_eleven.cpp): the catalog columns above plus the runtime',
        ' * AVAILABILITY flag, which is factory state (factory != null), not',
        ' * catalog data. */',
        'typedef struct GxFxEffectInfo {',
        '    const char *key;',
        '    const char *label;',
        '    const char *category;',
        '    int stereo;            /* 1 = native stereo path, 0 = dual-mono */',
        '    int memory;            /* 1 = MB-scale buffers at create/activate */',
        '    int param_count;       /* live params */',
        '    int available;         /* 1 = factory live (gxfx_create can succeed) */',
        '} GxFxEffectInfo;',
        '',
        '/* Per-effect param tables in id order. */',
        '',
    );

    effects.forEach((fx, i) => {
        const table = `GXFX_${fx.key.toUpperCase()}_PARAMS`;
        if (fx.params.length === 0) {
            // Zero-param effect (envelope-driven): param_count 0 hides the
            // placeholder row; ISO C++ forbids zero-length arrays.
            push(`static const GxFxParamDef ${table}[1] = {`);
            push('    { 0, "", "", 0.0f, 0.0f, 0.0f, 0.0f, 0 },');
            push('};');
            push('');
            return;
        }
        const rows = fx.params.map((p) => ([
            { s: `${p.port},`, right: true },
            { s: `"${p.symbol}",`, right: false },
            { s: `"${p.name}",`, right: false },
            { s: `${fC(p.default)},`, right: true },
            { s: `${fC(p.min)},`, right: true },
            { s: `${fC(p.max)},`, right: true },
            { s: `${fC(p.step)},`, right: true },
            { s: `${p.integer ? 1 : 0}`, right: false },
        ]));
        const widths = columnWidths(rows);
        push(`static const GxFxParamDef ${table}[${fx.params.length}] = {`);
        if (i === 0) push('    /* port  symbol    name     def     min      max    step  int */');
        for (const r of rows) push(`    ${layoutRow(r, widths)}`);
        push('};');
        push('');
    });

    push('/* Effect catalog in id order (row index = effect id). */');
    push('');
    const rows = effects.map((fx) => ([
        { s: `"${fx.key}",`, right: false },
        { s: `"${fx.label}",`, right: false },
        { s: `"${fx.category}",`, right: false },
        { s: `${fx.stereo ? 1 : 0},`, right: true },
        { s: `${fx.memory ? 1 : 0},`, right: true },
        { s: `${fx.params.length},`, right: true },
        { s: `GXFX_${fx.key.toUpperCase()}_PARAMS`, right: false },
    ]));
    const widths = columnWidths(rows);
    push(`static const GxFxEffectDef GXFX_CATALOG[GXFX_EFFECT_COUNT] = {`);
    push('    /* id  key  label  category  stereo memory count params */');
    effects.forEach((fx, i) => {
        push(`    /* ${String(fx.id).padStart(2)} */ ${layoutRow(rows[i], widths)}`);
    });
    push('};');
    push('');

    push('/*');
    push(' * Padded host tables (gxfx_defaults.h consumption pattern): defaults in');
    push(' * ENGINE units, rows padded to GXFX_MAX_PARAMS with 0.0f; PortIndex per');
    push(' * param ordinal, rows zero-padded beyond param_count.');
    push(' */');
    push('');
    push(`static const float FX_DEFAULTS[GXFX_EFFECT_COUNT][GXFX_MAX_PARAMS] = {`);
    effects.forEach((fx) => {
        const row = [];
        for (let p = 0; p < FX_SLOT_PARAMS; p++) {
            row.push(p < fx.params.length ? fC(fx.params[p].default) : fC(0));
        }
        push(`    /* ${String(fx.id).padStart(2)}: ${fx.key} — ${fx.params.length} params + ${FX_SLOT_PARAMS - fx.params.length} pad */`);
        push('    {');
        for (let i = 0; i < row.length; i += 8) {
            push('        ' + row.slice(i, i + 8).map((v) => v.padStart(8)).join(',') + ',');
        }
        push('    },');
    });
    push('};');
    push('');
    push(`static const int FX_PORTS[GXFX_EFFECT_COUNT][GXFX_MAX_PARAMS] = {`);
    effects.forEach((fx) => {
        push(`    /* ${String(fx.id).padStart(2)}: ${fx.key} */`);
        push(`    { ${fx.params.map((p) => p.port).join(', ') || '0'} },`);
    });
    push('};');
    push('');
    push('#endif /* GXFX_CATALOG_H */');
    push('');

    return L.join('\n');
}

// ---------------------------------------------------------------------------
// --check diff (bounded unified-style snippet around the first difference)
// ---------------------------------------------------------------------------

function diffSnippet(committed, regen) {
    const al = committed.split('\n');
    const bl = regen.split('\n');
    let p = 0;
    while (p < al.length && p < bl.length && al[p] === bl[p]) p++;
    let s = 0;
    while (s < al.length - p && s < bl.length - p && al[al.length - 1 - s] === bl[bl.length - 1 - s]) s++;
    const ctx = 3;
    const lo = Math.max(0, p - ctx);
    const endA = al.length - s + ctx;
    const endB = bl.length - s + ctx;
    const lines = [`@@ around line ${lo + 1} @@`];
    for (let i = lo; i < p; i++) lines.push(`  ${al[i]}`);
    for (let i = p; i < Math.min(endA, p + 40); i++) lines.push(`- ${al[i]}`);
    for (let i = p; i < Math.min(endB, p + 40); i++) lines.push(`+ ${bl[i]}`);
    if (endA - p > 40 || endB - p > 40) lines.push('  ... (diff truncated)');
    return lines.join('\n');
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

function main() {
    const { raw, problems } = loadSpec();
    const spec = normalize(raw, problems);
    if (problems.length > 0) {
        console.error(`gxfx param spec failed validation — refusing to generate:`);
        for (const p of problems) console.error(`  - ${p}`);
        process.exit(2);
    }
    const maxParams = Math.max(...spec.effects.map((fx) => fx.params.length));

    const outputs = [
        { rel: OUT_H, content: genHeader(spec) },
    ];

    if (process.argv.includes('--check')) {
        const stale = [];
        for (const { rel, content } of outputs) {
            let committed = null;
            try {
                committed = readFileSync(join(ROOT, rel), 'utf8');
            } catch {
                stale.push(rel);
                continue;
            }
            if (committed !== content) stale.push(rel);
        }
        if (stale.length > 0) {
            console.error('STALE generated outputs (regenerate with `node build/gen-gxfx-params.mjs`):');
            for (const { rel, content } of outputs) {
                if (!stale.includes(rel)) continue;
                console.error(`  ${rel}`);
                try {
                    const committed = readFileSync(join(ROOT, rel), 'utf8');
                    console.error(diffSnippet(committed, content)
                        .split('\n').map((l) => `    ${l}`).join('\n'));
                } catch { /* missing file — no diff to show */ }
            }
            process.exit(1);
        }
        console.log(`OK — all ${outputs.length} generated outputs are fresh`);
        return;
    }

    for (const { rel, content } of outputs) {
        const abs = join(ROOT, rel);
        writeFileSync(abs, content);
        console.log(`wrote ${rel}`);
    }
    console.log(
        `gxfx catalog: ${spec.effects.length} effects, widest ${maxParams} params ` +
        `(padded row ${FX_SLOT_PARAMS}, slots ${FX_SLOTS})`,
    );
}

main();
