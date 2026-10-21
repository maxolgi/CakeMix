// gxfx_convolver.cpp — the partitioned convolution core (see
// gxfx_convolver.h for the design notes). Port of OctOBX
// wasm/obxd/gxfx_convolver.cpp with two CakeMix adaptations:
//   - kissfft is NOT compiled here: gxfx_kissfft.cpp is the archive's sole
//     compilation owner of kiss_fft.c/kiss_fftr.c (extern "C"); this TU
//     links the kiss_* C symbols in-archive and includes only the public
//     headers via gxfx_convolver.h.
//   - storage is the archive's C malloc/free floor instead of std::vector
//     (the freestanding <vector> shim is gxfx_mb.cpp-private; shapes and
//     semantics are unchanged — see the header's allocation policy). The
//     zita BufferResampler's new[]'d output is delete[]'d at the boundary
//     (eleven's operator delete[] floor); the history slide is a fixed
//     forward copy loop (dst < src, non-overlapping) instead of memmove so
//     the object's libcall surface stays tight.
#include "gxfx_convolver.h"

#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>

#include "gx_resampler.h" // via -I vendor/guitarix/trunk/src/LV2/DSP
                         // (implementation link-complete from
                         // gxfx_dsp_distortion.cpp)

namespace gxfx_conv {

PartitionedConvolver::PartitionedConvolver()
    : ir_src_(0), ir_src_len_(0), ir_rate_(48000.0), engine_rate_(48000.0),
      rebuild_pending_(false), fd_partitions_(0), parts_(0), fd_ring_(0),
      slot_(0), cfg_fwd_(0), cfg_inv_(0) {
    memset(hist_, 0, sizeof(hist_));
    memset(acc_, 0, sizeof(acc_));
    memset(blk_time_, 0, sizeof(blk_time_));
}

PartitionedConvolver::~PartitionedConvolver() {
    if (cfg_fwd_) kiss_fftr_free(cfg_fwd_);
    if (cfg_inv_) kiss_fftr_free(cfg_inv_);
    free(fd_partitions_);
    free(fd_ring_);
    free(ir_src_);
}

void PartitionedConvolver::ensure_cfgs() {
    if (!cfg_fwd_) cfg_fwd_ = kiss_fftr_alloc(FFT_N, 0, 0, 0);
    if (!cfg_inv_) cfg_inv_ = kiss_fftr_alloc(FFT_N, 1, 0, 0);
}

void PartitionedConvolver::set_ir(const float* ir, size_t len,
                                  double ir_rate, double engine_rate) {
    if (!ir || len == 0) {
        free(ir_src_);
        ir_src_ = 0;
        ir_src_len_ = 0;
        parts_ = 0;
        rebuild_pending_ = false;
        reset();
        return;
    }
    float* nu = (float*)malloc(len * sizeof(float));
    if (!nu) return; // allocation failure: keep the previous IR running
    memcpy(nu, ir, len * sizeof(float));
    free(ir_src_);
    ir_src_ = nu;
    ir_src_len_ = len;
    ir_rate_ = ir_rate;
    engine_rate_ = engine_rate;
    rebuild_pending_ = true; // applied on the next process()
}

void PartitionedConvolver::reset() {
    memset(hist_, 0, sizeof(hist_));
    if (fd_ring_ && parts_)
        memset(fd_ring_, 0, parts_ * BINS * sizeof(kiss_fft_cpx));
    slot_ = 0;
}

void PartitionedConvolver::rebuild() {
    rebuild_pending_ = false;
    ensure_cfgs();
    if (!cfg_fwd_ || !cfg_inv_ || !ir_src_ || ir_src_len_ == 0) {
        parts_ = 0;
        return;
    }

    // Effective IR at engine rate: use the source directly when the rates
    // match (the cabinet wrapper pre-resamples + bakes its impulse-former
    // EQ at engine rate), else resample via zita (gx_resample::
    // BufferResampler — implementation link-complete from the
    // gxfx_dsp_distortion.cpp TU). process() takes a mutable input; ir_src_
    // is our own copy, and its new[]'d result is delete[]'d once the
    // partition FFTs below have consumed it.
    const float* ir_eff = ir_src_;
    size_t len = ir_src_len_;
    float* resampled = 0;
    if (ir_rate_ != engine_rate_ && engine_rate_ > 0.0) {
        gx_resample::BufferResampler r;
        int32_t olen = 0;
        float* out = r.process((int32_t)ir_rate_, (int32_t)ir_src_len_,
                               ir_src_, (int32_t)engine_rate_, &olen);
        if (!out || olen <= 0) {
            parts_ = 0;
            return;
        }
        resampled = out;
        len = (size_t)olen;
        ir_eff = resampled;
    }

    parts_ = (len + BLOCK - 1) / BLOCK;
    if (parts_ == 0) {
        delete[] resampled;
        return;
    }

    // Frequency-domain partitions: H_p = FFT([h[pB..(p+1)B) zero-padded to
    // N]). Fresh (zeroed) streaming state for the new IR; the old buffers
    // are replaced in one go so a mid-rebuild failure keeps a valid state.
    kiss_fft_cpx* parts =
        (kiss_fft_cpx*)malloc(parts_ * BINS * sizeof(kiss_fft_cpx));
    kiss_fft_cpx* ring =
        (kiss_fft_cpx*)malloc(parts_ * BINS * sizeof(kiss_fft_cpx));
    if (!parts || !ring) { // allocation failure: fail-open passthrough
        free(parts);
        free(ring);
        delete[] resampled;
        parts_ = 0;
        return;
    }
    memset(parts, 0, parts_ * BINS * sizeof(kiss_fft_cpx));
    memset(ring, 0, parts_ * BINS * sizeof(kiss_fft_cpx));
    free(fd_partitions_);
    fd_partitions_ = parts;
    free(fd_ring_);
    fd_ring_ = ring;

    float tmp[FFT_N];
    for (size_t p = 0; p < parts_; ++p) {
        memset(tmp, 0, sizeof(tmp));
        const size_t chunk = (len - p * BLOCK < BLOCK) ? (len - p * BLOCK) : BLOCK;
        if (chunk) memcpy(tmp, ir_eff + p * BLOCK, chunk * sizeof(float));
        kiss_fftr(cfg_fwd_, tmp, &fd_partitions_[p * BINS]);
    }
    delete[] resampled;

    reset();
}

void PartitionedConvolver::run_block(float* io) {
    // 1. Slide the history window: hist = [oldest B | newest B]. Fixed
    //    forward copy (dst < src, ranges cannot overlap).
    for (size_t i = 0; i < BLOCK; ++i)
        hist_[i] = hist_[BLOCK + i];
    memcpy(hist_ + BLOCK, io, BLOCK * sizeof(float));

    // 2. Forward transform of the window; store as the newest ring slot.
    kiss_fftr(cfg_fwd_, hist_, &fd_ring_[slot_ * BINS]);

    // 3. Accumulate H_p * X_{p blocks ago} over all partitions.
    memset(acc_, 0, sizeof(acc_));
    for (size_t p = 0; p < parts_; ++p) {
        const kiss_fft_cpx* h = &fd_partitions_[p * BINS];
        const size_t xpos = (slot_ + parts_ - p) % parts_;
        const kiss_fft_cpx* x = &fd_ring_[xpos * BINS];
        for (size_t b = 0; b < BINS; ++b) {
            acc_[b].r += h[b].r * x[b].r - h[b].i * x[b].i;
            acc_[b].i += h[b].r * x[b].i + h[b].i * x[b].r;
        }
    }

    // 4. Inverse transform; the valid (alias-free) half is the LAST B
    //    samples of the N-point circular result. kissfft's transforms are
    //    unscaled, so divide by N exactly once.
    kiss_fftri(cfg_inv_, acc_, blk_time_);
    const float scale = 1.0f / (float)FFT_N;
    for (size_t i = 0; i < BLOCK; ++i)
        io[i] = blk_time_[BLOCK + i] * scale;

    slot_ = (slot_ + 1) % parts_;
}

void PartitionedConvolver::process(size_t n, float* x) {
    if (rebuild_pending_) rebuild();
    if (parts_ == 0 || !x) return; // disabled / passthrough
    size_t off = 0;
    for (; off + (size_t)BLOCK <= n; off += (size_t)BLOCK)
        run_block(x + off);
    // Sub-block remainder (no caller): left untouched in place.
}

} // namespace gxfx_conv
