// gxfx_dsp_tuner.cpp — "Tuner" (catalog id 81, category special) as
// freestanding object code for the CakeMix mixer engine
// (crates/mixer-wasm): gxtuner.lv2's pitch tracker
// (gx_pitch_tracker.{h,cpp} + tuner.cc's PluginLV2 adapter, unmodified,
// namespace-wrapped like every gxfx DSP include). The tracker is the NSDF
// (normalized square difference function) pitch detector guitarix credits
// to K4Guitune/tartini: input is resampled to a fixed 20.5 kHz ring
// (gx_resample's Resampler, compiled into gxfx_dsp_distortion.cpp), and
// every TRACKER_PERIOD (0.1 s) upstream copies the ring and sem_post()s a
// dedicated analysis thread which runs one FFT-sized autocorrelation via
// an r2r FFT pair.
//
// Ported from OctOBX wasm/obxd/gxfx_dsp_tuner.cpp (the authoritative
// wrapper TU); namespaces, PortIndex enum and factory are VERBATIM.
// Adaptations for this repo, each mechanical:
//   * include paths (vendor submodule depth, gx_pluginlv2.h/gx_resampler.h
//     via -I vendor/guitarix/trunk/src/LV2/DSP);
//   * the three OctOBX blocker shims live in THIS repo as gxfx/fftw3.h
//     (forwarder onto gxfx/shim/fftw3.h — r2r halfcomplex fftw subset over
//     kissfft, UNSCALED round trip), gxfx/semaphore.h (sem_wait gate
//     macro) and gxfx/sigc++/sigc++.h (no-op signal); gxfx/ is on BOTH
//     targets' include paths so the vendored angle-includes inside
//     gx_pitch_tracker.h resolve identically everywhere (native builds
//     never see glibc's real <semaphore.h> — a real sem_wait would block
//     forever inside run()'s for(;;));
//   * gxfx/gxtuner_pthread_shim.h grew a __wasm__ type/attr block: OctOBX
//     leaned on emscripten's REAL <pthread.h> for pthread_t/
//     pthread_attr_t/sched_param, but this repo's freestanding
//     gxfx/shim/pthread.h carries only zita's no-op mutex surface;
//   * log2f is declared below before the vendor includes: the freestanding
//     gxfx/shim/cmath subset stops short of it, and the wasm link resolves
//     it from the Rust math floor (src/gxfx.rs math_shim!). Natively
//     glibc's <cmath> (via gxfx_prelude.h) declares the same signature.
//
// Blockers neutered by the shims (NO guitarix source edits):
//   - fftw: the tracker plans one R2HC + one HC2R r2r transform
//     (m_fftSize = 3072 at the fixed FFT_SIZE 2048 window). The
//     gxfx/shim/fftw3.h implements both over kiss_fftr with fftw's exact
//     halfcomplex layout and UNNORMALIZED round trip (the tracker divides
//     the HC2R output by n itself, and derives its NSDF normalizer from
//     the same output — a scaled shim would corrupt it).
//   - pthread+semaphore: upstream's analysis thread is spawned from
//     PitchTracker::init and loops forever on sem_wait(). In-tree the
//     thread is NEVER spawned: gxfx/gxtuner_pthread_shim.h's
//     pthread_create RECORDS the entry (always PitchTracker::static_run +
//     the tracker this) and this wrapper captures the pair per instance
//     at init time (duplicate tuner slots each drive their own tracker),
//     then re-invokes it on a ~100 ms block cadence; gxfx/semaphore.h's
//     sem_wait gate macro turns each run() invocation into EXACTLY ONE
//     analysis pass (first sem_wait falls through into the analysis body;
//     when the for(;;) loops back, the gate returns). Tens of µs per pass
//     on the audio thread.
//   - sigc++/MIDI/uniBar: dropped entirely. Upstream's sigc new_freq
//     signal feeds the LV2 wrapper's MIDI-note-out machinery (play_midi,
//     metronome, uniBar) — none of it is an insert effect. The no-op
//     gxfx/sigc++/sigc++.h keeps the member callable; the detected
//     frequency reaches the UI through the engine's out-port surface
//     instead: the FREQ control OUTPUT port (PortIndex 0, ttl index 0) —
//     the rack (src/gxfx.rs) connects it to the slot's out-port row and
//     the UI polls fx_out_param (looper bar1..4 precedent).
//
// Param surface (GUI-less sensible subset of gx_tuner.ttl, inline in the
// generator manifest — the ttl's other control inputs are MIDI machinery
// CHANNEL/ONMIDI/FASTNOTE/PITCHBEND/SINGLENOTE/BPM/VELOCITY/VERIFY/GATE/
// SYNTHFREQ/GAIN, GUI-only TUNER_MODE/TEMPERAMENT/MAXL/RESET, and the
// MIDI/note enum notes; note the ttl's tail port order also disagrees with
// gxtuner.h's PortIndex, so the ttl cannot be trusted for indexes anyway):
//   port 1 REFFREQ   427..453 Hz (default 440) — display reference: the
//                    engine never reads it (upstream's wrapper doesn't
//                    either — note naming is client-side); the UI's note
//                    math shifts A4 by it.
//   port 4 THRESHOLD -60..4 dB (default -50) — db2power gate for the
//                    tracker, applied with upstream's 0.1 dB hysteresis.
//
// Audio is BIT-IDENTICAL through the tuner (analysis reads `input` only);
// mono (one audio input) → dual-mono host, so L and R each run a tracker
// and both write the slot's shared FREQ out slot (same signal, last-writer
// per block — harmless).
//
// ODR: gxfx_dsp_eleven.cpp owns the global operator new/delete overloads
// and the memset definition for the whole gxfx archive (the tracker's
// new'd m_buffer/m_input resolve against them at archive link time); this
// TU defines NONE of them, no extern "C", no emscripten. NOT yet wired
// into gxfx_dsp_eleven.cpp's factory table (row 81 stays null until the
// final wave's honesty-test flip; until then this object rides the
// archive unreferenced).
//
// Standalone syntax check (native):
//   clang++ -std=c++20 -O2 -fsyntax-only -Wno-vla-cxx-extension \
//     -I crates/mixer-wasm/gxfx \
//     -I vendor/guitarix/trunk/src/LV2/DSP \
//     -I vendor/guitarix/trunk/src/zita-resampler-1.1.0 \
//     crates/mixer-wasm/gxfx/gxfx_dsp_tuner.cpp
#include "gxfx_prelude.h"

// Shims FIRST and at GLOBAL scope: tuner.cc pulls gx_pitch_tracker.h from
// inside namespace gxt below, and the shim include guards must already
// have fired there so no shim content leaks into the namespace.
#include <fftw3.h>              // gxfx/shim/fftw3.h (via shim dir / gxfx/fftw3.h forwarder) — r2r halfcomplex subset
#include "gxtuner_pthread_shim.h" // real <pthread.h> + recording overrides (+ __wasm__ types)
#include <semaphore.h>          // gxfx/semaphore.h — sem_wait gate macro
#include <sigc++/sigc++.h>      // gxfx/sigc++/sigc++.h — no-op signal

// get_estimated_note()'s log2f: missing from the freestanding
// gxfx/shim/cmath subset, provided natively by glibc and at wasm link
// time by the Rust math floor (src/gxfx.rs math_shim! → libm::log2f).
extern "C" float log2f(float x);

#include <assert.h>

#include "gx_pluginlv2.h"  // via -I vendor/guitarix/trunk/src/LV2/DSP
#include "gx_resampler.h"  // via -I vendor/guitarix/trunk/src/LV2/DSP

namespace gxt {
// The guitarix tuner adapter (class tuner — owns the PitchTracker, maps
// the PluginLV2 surface onto it) and the tracker itself, unmodified.
#include "../../../vendor/guitarix/trunk/src/LV2/gxtuner.lv2/tuner.cc"
#include "../../../vendor/guitarix/trunk/src/LV2/gxtuner.lv2/gx_pitch_tracker.cpp"
} // namespace gxt

namespace gxfx_tuner {

// Host render quantum — the only block size the mixer engine renders.
enum { BUFSIZE = 128 };

// Port space (subset of gxtuner.h's PortIndex, ttl-consistent indexes).
enum {
    FREQ = 0,       // control OUTPUT — detected frequency in Hz (0 = silent)
    REFFREQ = 1,    // control input — display reference pitch (unused here)
    THRESHOLD = 4,  // control input — tracker gate in dB
};

class TunerDsp : public PluginLV2 {
private:
    gxt::tuner* t_;         // the adapter (owns the PitchTracker)
    void* (*run_fn_)(void*); // recorded "thread" entry (PitchTracker::static_run —
    void* run_arg_;          //  exact pthread signature; wasm checks indirect-call types)
    float* freq_ptr_;       // connected FREQ out port → slot out-port row
    float* threshold_ptr_;  // connected THRESHOLD param mirror slot (dB)
    float threshold_;       // last applied threshold (0.1 dB hysteresis band)
    int blocks_;            // blocks since the last analysis pass
    int cadence_;           // blocks per pass — ~(rate/10)/BUFSIZE ≈ 37 @ 48k

public:
    TunerDsp()
        : PluginLV2(),
          t_(static_cast<gxt::tuner*>(gxt::plugin())),
          run_fn_(0), run_arg_(0),
          freq_ptr_(0), threshold_ptr_(0), threshold_(0.0f),
          blocks_(0), cadence_(1) {
        version = PLUGINLV2_VERSION;
        id = "tuner";
        name = N_("Tuner");
        mono_audio = compute_static;
        stereo_audio = 0;
        set_samplerate = init_static;
        activate_plugin = activate_static;
        connect_ports = connect_static;
        delete_instance = del_instance;
    }
    ~TunerDsp() {
        if (t_ && t_->delete_instance) t_->delete_instance(t_);
    }

    // tuner.cc's init (assigned to the adapter's set_samplerate) runs
    // PitchTracker::init → setParameters → start_thread → the pthread shim
    // RECORDS the entry instead of spawning. Capture the pair per instance
    // (duplicate tuner slots must never share a recorded tracker), and set
    // the analysis cadence from the rate.
    static void init_static(uint32_t rate, PluginLV2* p) {
        TunerDsp* s = static_cast<TunerDsp*>(p);
        if (!s->t_) return;
        s->t_->set_samplerate(rate, s->t_);
        s->run_fn_ = gxfx_pthread_entry;
        s->run_arg_ = gxfx_pthread_arg;
        s->cadence_ = (int)(rate / 10u / BUFSIZE); // ~100 ms of blocks
        if (s->cadence_ < 1) s->cadence_ = 1;
        s->blocks_ = 0;
    }

    static void connect_static(uint32_t port, void* data, PluginLV2* p) {
        TunerDsp* s = static_cast<TunerDsp*>(p);
        if (port == FREQ) s->freq_ptr_ = static_cast<float*>(data);
        else if (port == THRESHOLD) s->threshold_ptr_ = static_cast<float*>(data);
        // REFFREQ (port 1) is display-only — the adapter has no params to
        // forward (upstream's wrapper never read it either).
    }

    static int activate_static(bool start, PluginLV2* p) {
        TunerDsp* s = static_cast<TunerDsp*>(p);
        if (s->t_ && s->t_->activate_plugin) s->t_->activate_plugin(start, s->t_);
        return 0; // activate(false) → PitchTracker::reset() upstream
    }

    static void compute_static(int count, float* input, float* output, PluginLV2* p) {
        TunerDsp* s = static_cast<TunerDsp*>(p);
        // Feed the tracker — feed_tuner READS `input` and ignores its
        // output arg; the samples are never modified (tuner = transparent).
        if (s->t_ && s->t_->mono_audio) s->t_->mono_audio(count, input, input, s->t_);
        if (output != input) memcpy(output, input, (size_t)count * sizeof(float));
        // THRESHOLD gate with upstream's 0.1 dB hysteresis (threshold_ = 0
        // init means the ttl default -50 applies on the first block, like
        // upstream's uninitialized-then-differs read).
        if (s->threshold_ptr_) {
            const float v = *s->threshold_ptr_;
            if (fabsf(v - s->threshold_) > 0.1f) {
                s->threshold_ = v;
                gxt::tuner::set_threshold_level(*s->t_, v);
            }
        }
        // ~100 ms cadence: one inline NSDF analysis pass (level check +
        // 3072-pt FFT pair + maxima search — tens of µs). The sem gate
        // makes the recorded run() entry analyze once, then return.
        if (s->run_fn_ && ++s->blocks_ >= s->cadence_) {
            s->blocks_ = 0;
            gxfx_sem_reset();
            s->run_fn_(s->run_arg_);
        }
        // FREQ out port, refreshed every block like upstream's
        // `*(freq) = self.get_freq(self)` (0 until the first detection).
        if (s->freq_ptr_) *s->freq_ptr_ = gxt::tuner::get_freq(*s->t_);
    }

    static void del_instance(PluginLV2* p) { delete static_cast<TunerDsp*>(p); }
};

PluginLV2* create() { return new TunerDsp(); }

} // namespace gxfx_tuner

// namespace path: gxfx_tuner::create(). Factory order MUST match the
// generator manifest order (id 81 tuner, after id 80 detune).
typedef PluginLV2* (*gxfx_factory)();
PluginLV2* gxfx_create_tuner() { return gxfx_tuner::create(); }
