/*
 * crates/mixer-wasm/gxfx/gxfx_catalog.h — guitarix FX catalog for the
 * CakeMix mixer engine (per-strip insert chains).
 *
 * Data transcribed from the source-of-truth generated TypeScript table
 * (OctOBX src/gxfx-params.ts, FX_EFFECTS ids 0..10 — the canonical
 * eleven-effect chain in manifest order). Hand-maintained here until the
 * generator lands; every value must stay identical to the TS table.
 *
 * Layout (modeled on OctOBX wasm/obxd/gxfx_defaults.h, extended to carry
 * the full per-param UI metadata for the future JSON surface):
 *   GXFX_EFFECT_COUNT  catalog size; row index = effect id (0..10).
 *   GXFX_CHAIN_SLOTS   FX slots per insert chain.
 *   GXFX_MAX_PARAMS    padded per-effect param row width (48).
 *   GxFxParamDef       per-param: LV2 PortIndex (`port`, the value passed
 *                      to fx_set_param), symbol, display name, and
 *                      default / min / max / step in ENGINE units (ttl
 *                      ranges, NOT 0..1), plus the integer flag.
 *   GxFxEffectDef      per-effect: key, label, menu category, stereo flag
 *                      (1 = native stereo path, 0 = dual-mono), live
 *                      param count, param table.
 *   GXFX_CATALOG       the 11 effects in id order; param order = port
 *                      order. Param counts 2,1,3,5,4,6,3,4,7,7,5.
 *   FX_DEFAULTS[id][p] per-effect defaults, padded to 48 with 0.0f.
 *   FX_PORTS[id][p]    LV2 PortIndex per param ordinal, zero-padded.
 *
 * distortion (id 2) keeps its 3-param row even though its DSP TU lands
 * separately.
 */
#ifndef GXFX_CATALOG_H
#define GXFX_CATALOG_H

#define GXFX_EFFECT_COUNT 11
#define GXFX_CHAIN_SLOTS 11
#define GXFX_MAX_PARAMS 48

typedef struct GxFxParamDef {
    int port;              /* LV2 PortIndex passed to fx_set_param */
    const char *symbol;    /* LV2 port symbol */
    const char *name;      /* display name */
    float def;             /* default, ENGINE units (ttl range, not 0..1) */
    float min;
    float max;
    float step;
    int is_integer;        /* 1 = integer-stepped */
} GxFxParamDef;

typedef struct GxFxEffectDef {
    const char *key;
    const char *label;
    const char *category;  /* menu category string */
    int stereo;            /* 1 = native stereo path, 0 = dual-mono */
    int param_count;       /* live params; rest of each row is padding */
    const GxFxParamDef *params;
} GxFxEffectDef;

/* Per-effect param tables in id order. */

static const GxFxParamDef GXFX_WAH_PARAMS[2] = {
    /* port  symbol    name     def     min      max    step  int */
    { 2, "VOLUME",  "Volume",  0.0f, -20.0f,    4.0f,  0.1f,  0 },
    { 3, "HOTPOTZ", "Wah",     0.5f,   0.0f,    1.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_OVERDRIVE_PARAMS[1] = {
    { 2, "SCREAM",  "Scream",  0.5f,   0.0f,    1.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_DISTORTION_PARAMS[3] = {
    { 2, "LEVEL",   "Level",  -2.0f, -20.0f,   12.0f,  0.1f,  0 },
    { 3, "TONE",    "Tone",    0.5f,   0.0f,    1.0f,  0.01f, 0 },
    { 4, "DRIVE",   "Drive",   0.5f,   0.0f,    1.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_COMPRESSOR_PARAMS[5] = {
    { 0, "RATIO",     "Ratio",     2.0f,    1.0f,  20.0f,  0.1f,   0 },
    { 1, "KNEE",      "Knee",      3.0f,    0.0f,  20.0f,  0.1f,   0 },
    { 2, "THRESHOLD", "Threshold", -20.0f, -96.0f,  10.0f,  0.1f,  0 },
    { 3, "RELEASE",   "Release",   0.5f,    0.0f,  10.0f,  0.01f,  0 },
    { 4, "ATTACK",    "Attack",    0.002f,  0.0f,   1.0f,  0.001f, 0 },
};

static const GxFxParamDef GXFX_CHORUS_PARAMS[4] = {
    { 0, "level", "Level",  0.5f,  0.0f,   1.0f,  0.01f, 0 },
    { 1, "delay", "Delay",  0.02f, 0.0f,   0.2f,  0.01f, 0 },
    { 2, "depth", "Depth",  0.02f, 0.0f,   1.0f,  0.01f, 0 },
    { 3, "freq",  "Freq",   3.0f,  0.1f,  10.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_FLANGER_PARAMS[6] = {
    { 0, "depth",    "Depth",    0.5f,    0.0f,    5.0f,  0.01f, 0 },
    { 1, "width",    "Width",    5.0f,    0.0f,   10.0f,  0.01f, 0 },
    { 2, "freq",     "Freq",     0.2f,    0.05f,  10.0f,  1.06f, 0 },
    { 3, "feedback", "Feedback", -0.707f, -0.99f,  0.99f, 0.01f, 0 },
    { 4, "dry_wet",  "Wet",    100.0f,    0.0f,  100.0f,  1.0f,  0 },
    { 5, "mix",      "Mix",      0.0f,   -1.0f,    1.0f,  0.1f,  0 },
};

static const GxFxParamDef GXFX_PHASER_PARAMS[3] = {
    { 0, "dry_wet", "Dry/Wet", 50.0f,   0.0f, 100.0f,  1.0f,  0 },
    { 1, "level",   "Level",    0.0f, -60.0f,  10.0f,  0.1f,  0 },
    { 2, "speed",   "Speed",    0.5f,   0.0f,  10.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_TREMOLO_PARAMS[4] = {
    { 0, "dry_wet", "Dry/Wet", 50.0f,  0.0f, 100.0f,  1.0f,  0 },
    { 1, "mode",    "Mode",     0.0f,  0.0f,   1.0f,  1.0f,  1 },
    { 2, "depth",   "Depth",    0.5f,  0.0f,   1.0f,  0.01f, 0 },
    { 3, "freq",    "Freq",     5.0f,  0.1f,  50.0f,  0.1f,  0 },
};

static const GxFxParamDef GXFX_DELAY_PARAMS[7] = {
    { 0, "mode",    "Mode",        0.0f,   0.0f,    1.0f,  1.0f,  1 },
    { 1, "level_r", "R Level",   -10.0f, -20.0f,   20.0f,  0.1f,  0 },
    { 2, "timt_r",  "R Time",   1000.0f,   1.0f, 5000.0f, 10.0f,  0 },
    { 3, "level_l", "L Level",   -10.0f, -20.0f,   20.0f,  0.1f,  0 },
    { 4, "timt_l",  "L Time",   1000.0f,   1.0f, 5000.0f, 10.0f,  0 },
    { 5, "lfo",     "LFO",         0.2f,   0.2f,    5.0f,  0.01f, 0 },
    { 6, "link",    "Link (L+R)",  0.0f,   0.0f,    1.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_ECHO_PARAMS[7] = {
    { 0, "mode",    "Mode",        0.0f,  0.0f,    1.0f,  1.0f,  1 },
    { 1, "level_r", "R Level",    30.0f,  0.0f,  100.0f,  0.1f,  0 },
    { 2, "timt_r",  "R Time",    100.0f,  1.0f, 2000.0f,  1.0f,  0 },
    { 3, "level_l", "L Level",    30.0f,  0.0f,  100.0f,  0.1f,  0 },
    { 4, "timt_l",  "L Time",    100.0f,  1.0f, 2000.0f,  1.0f,  0 },
    { 5, "lfo",     "LFO",         0.2f,  0.2f,    5.0f,  0.01f, 0 },
    { 6, "link",    "Link (L+R)",  0.0f,  0.0f,    1.0f,  0.01f, 0 },
};

static const GxFxParamDef GXFX_REVERB_PARAMS[5] = {
    { 0, "dry_wet",  "Dry/Wet",  50.0f,  0.0f, 100.0f,  1.0f,   0 },
    { 1, "lfo",      "LFO",       0.2f,  0.2f,   5.0f,  0.01f,  0 },
    { 2, "roomsize", "Roomsize",  0.5f,  0.0f,   1.0f,  0.025f, 0 },
    { 3, "damp",     "Damp",      0.2f,  0.0f,   1.0f,  0.025f, 0 },
    { 4, "mode",     "Mode",      0.0f,  0.0f,   1.0f,  1.0f,   1 },
};

/* Effect catalog in id order (row index = effect id). */

static const GxFxEffectDef GXFX_CATALOG[GXFX_EFFECT_COUNT] = {
    /* id  key          label         category      stereo count params */
    {     "wah",        "Wah",        "wah",         0,     2,    GXFX_WAH_PARAMS },
    {     "overdrive",  "Overdrive",  "drive",       0,     1,    GXFX_OVERDRIVE_PARAMS },
    {     "distortion", "Distortion", "drive",       0,     3,    GXFX_DISTORTION_PARAMS },
    {     "compressor", "Compressor", "dynamics",    0,     5,    GXFX_COMPRESSOR_PARAMS },
    {     "chorus",     "Chorus",     "modulation",  1,     4,    GXFX_CHORUS_PARAMS },
    {     "flanger",    "Flanger",    "modulation",  0,     6,    GXFX_FLANGER_PARAMS },
    {     "phaser",     "Phaser",     "modulation",  0,     3,    GXFX_PHASER_PARAMS },
    {     "tremolo",    "Tremolo",    "modulation",  0,     4,    GXFX_TREMOLO_PARAMS },
    {     "delay",      "Delay",      "delay",       1,     7,    GXFX_DELAY_PARAMS },
    {     "echo",       "Echo",       "delay",       1,     7,    GXFX_ECHO_PARAMS },
    {     "reverb",     "Reverb",     "reverb",      1,     5,    GXFX_REVERB_PARAMS },
};

/*
 * Padded host tables (gxfx_defaults.h consumption pattern): defaults in
 * ENGINE units, rows padded to GXFX_MAX_PARAMS with 0.0f; PortIndex per
 * param ordinal, rows zero-padded beyond param_count.
 */

static const float FX_DEFAULTS[GXFX_EFFECT_COUNT][GXFX_MAX_PARAMS] = {
    /*  0: wah — 2 params + 46 pad */
    {
            0.0f,    0.5f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  1: overdrive — 1 params + 47 pad */
    {
            0.5f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  2: distortion — 3 params + 45 pad */
    {
           -2.0f,    0.5f,    0.5f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  3: compressor — 5 params + 43 pad */
    {
            2.0f,    3.0f,  -20.0f,    0.5f,  0.002f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  4: chorus — 4 params + 44 pad */
    {
            0.5f,   0.02f,   0.02f,    3.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  5: flanger — 6 params + 42 pad */
    {
            0.5f,    5.0f,    0.2f, -0.707f,  100.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  6: phaser — 3 params + 45 pad */
    {
           50.0f,    0.0f,    0.5f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  7: tremolo — 4 params + 44 pad */
    {
           50.0f,    0.0f,    0.5f,    5.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  8: delay — 7 params + 41 pad */
    {
            0.0f,  -10.0f, 1000.0f,  -10.0f, 1000.0f,    0.2f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /*  9: echo — 7 params + 41 pad */
    {
            0.0f,   30.0f,  100.0f,   30.0f,  100.0f,    0.2f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
    /* 10: reverb — 5 params + 43 pad */
    {
           50.0f,    0.2f,    0.5f,    0.2f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
            0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,    0.0f,
    },
};

static const int FX_PORTS[GXFX_EFFECT_COUNT][GXFX_MAX_PARAMS] = {
    /*  0: wah */
    { 2, 3 },
    /*  1: overdrive */
    { 2 },
    /*  2: distortion */
    { 2, 3, 4 },
    /*  3: compressor */
    { 0, 1, 2, 3, 4 },
    /*  4: chorus */
    { 0, 1, 2, 3 },
    /*  5: flanger */
    { 0, 1, 2, 3, 4, 5 },
    /*  6: phaser */
    { 0, 1, 2 },
    /*  7: tremolo */
    { 0, 1, 2, 3 },
    /*  8: delay */
    { 0, 1, 2, 3, 4, 5, 6 },
    /*  9: echo */
    { 0, 1, 2, 3, 4, 5, 6 },
    /* 10: reverb */
    { 0, 1, 2, 3, 4 },
};

#endif /* GXFX_CATALOG_H */
