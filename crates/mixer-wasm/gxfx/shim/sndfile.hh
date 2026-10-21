// gxfx/shim/sndfile.hh — no-op libsndfile stand-in for the guitarix
// livelooper (ported from OctOBX gxfx_shims/sndfile.hh, include-order
// shadow like fftw3.h).
//
// gx_livelooper.lv2's wrapper includes <sndfile.hh> (libsndfile's C++
// header, which pulls the C API); the bundle-LOCAL livelooper.cc actually
// references only the C surface: SF_INFO, SNDFILE, sf_open, sf_read_float,
// sf_close, sf_write_float, sf_write_sync. This stub implements exactly
// that surface with opens that ALWAYS FAIL (sf_open returns NULL), so:
//
//   - load_from_wave() skips the read (sf NULL) and returns 0 — loops
//     start EMPTY each session (fresh WASM heap, no persistence);
//   - save_to_wave() skips the write (sf NULL); sf_close(NULL) is a no-op;
//   - save_array()'s stat/mkdir dance may still run (getenv("HOME") under
//     an empty wasm environ returns NULL from the Rust c_floor stub, so
//     the path degrades to "" — harmless).
//
// Safe to shadow: unlike pthread.h (see the tuner's pthread shim —
// emcc's own headers include it), NO system header includes sndfile.hh.
// Include contract in THIS repo: gxfx_dsp_looper.cpp pulls it as
// "shim/sndfile.hh" (quoted) — native builds do not put shim/ on the
// include path, and the system libsndfile header must never be picked
// up (it would leave real sf_* symbols undefined at link time); the
// quoted include keeps the stub deterministic on BOTH targets.
#pragma once

#include <cstdint>

// nullptr (not NULL): emcc's header chain defined NULL transitively;
// the freestanding chain pulls in only <cstdint>, and this is C++.
typedef int64_t sf_count_t;

typedef struct SNDFILE_tag SNDFILE;

struct SF_INFO {
    sf_count_t frames;
    int samplerate;
    int channels;
    int format;
    int sections;
    int seekable;
};

enum {
    SFM_READ = 0x10,
    SFM_WRITE = 0x20,
};

enum {
    SF_FORMAT_WAV = 0x010000,
    SF_FORMAT_FLOAT = 0x0006,
};

inline SNDFILE* sf_open(const char*, int, SF_INFO*) { return nullptr; }
inline int sf_close(SNDFILE*) { return 0; }
inline sf_count_t sf_read_float(SNDFILE*, float*, sf_count_t) { return 0; }
inline sf_count_t sf_write_float(SNDFILE*, const float*, sf_count_t) { return 0; }
inline void sf_write_sync(SNDFILE*) {}
