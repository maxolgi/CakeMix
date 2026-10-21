// gxfx/sigc++/sigc++.h — CakeMix stand-in for libsigc++'s umbrella
// header inside the gxfx tuner TU, ported from OctOBX
// wasm/obxd/gxfx_shims/sigc++/sigc++.h. gx_pitch_tracker.h declares
// `sigc::signal<void> new_freq;` and fires it whenever the detected
// frequency changes; upstream the tuner LV2 wrapper connects a MIDI
// note-handler to it (gxtuner.cpp freq_changed_handler → play_midi).
//
// The port drops the whole sigc/MIDI/uniBar machinery: the wrapper polls
// the frequency through the engine's out-port surface (tuner FREQ, ttl
// port 0) instead of subscribing to a callback, so the signal only needs
// to be CALLABLE — invocation `new_freq()` must compile and do nothing.
//
// Minimal surface: a templated no-op signal<R> with operator(). The tuner
// TU is its only consumer (only gx_pitch_tracker.h includes sigc++);
// the file lives under gxfx/sigc++/ so the vendored angle-include
// `#include <sigc++/sigc++.h>` resolves identically on BOTH targets
// (gxfx/ is on every include list; there is no system libsigc++).
#pragma once

#ifndef GXFX_SIGCXX_SIGCPP_H
#define GXFX_SIGCXX_SIGCPP_H

namespace sigc {

template <typename R>
struct signal {
    // Fire the signal — no-op (no subscribers in the headless port).
    void operator()() const {}
};

} // namespace sigc

#endif // GXFX_SIGCXX_SIGCPP_H
