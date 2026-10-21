// gxfx/fftw3.h — angle-path forwarder onto gxfx/shim/fftw3.h (the fftw3
// API shim over the vendored kissfft; see that header). Vendored
// gx_pitch_tracker.h pulls `#include <fftw3.h>` from inside the gxfx_dsp_
// tuner TU's namespace block, and the angle search path differs per
// target: the freestanding wasm build puts gxfx/shim FIRST (-nostdinc)
// and resolves there directly, while NATIVE builds do not put gxfx/shim
// on the -I list at all — only gxfx/ itself (and the vendor dirs) are on
// both targets' include paths. This forwarder therefore lives one level
// up so `#include <fftw3.h>` resolves to the SAME shim everywhere and can
// never pick up a real system fftw3.h (whose fftwf_* symbols would be
// undefined at archive link time). Guard: the shim's own include guard
// fires through, so a double include (TU prelude + vendored header) is a
// no-op on both targets.
#pragma once

#include "shim/fftw3.h"
