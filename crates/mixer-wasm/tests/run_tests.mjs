#!/usr/bin/env node
/**
 * Test runner for CakeMix WASM mixer known-answer tests.
 * Run with: node tests/run_tests.mjs
 * (Build first with: wasm-pack build --target nodejs --release)
 */
import { MixerWasm } from '../pkg/mixer_wasm.js';

const SAMPLE_RATE = 48000;
const BLOCK_SIZE = 128;

function sineWave(freq, gain, n) {
    const buf = new Float32Array(n);
    for (let i = 0; i < n; i++) {
        buf[i] = gain * Math.sin(2 * Math.PI * freq * i / SAMPLE_RATE);
    }
    return buf;
}

let passed = 0;
let failed = 0;

function assert(cond, msg) {
    if (!cond) throw new Error(msg);
}

// Test 1: Basic sum of two sines (known-answer)
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sineA = sineWave(220, 0.5, BLOCK_SIZE);
    const sineB = sineWave(330, 0.5, BLOCK_SIZE);

    mixer.set_channel_input(0, sineA);
    mixer.set_channel_input(1, sineB);

    const output = mixer.process(BLOCK_SIZE);
    assert(output.length === BLOCK_SIZE * 2, `output length ${output.length} !== ${BLOCK_SIZE * 2}`);

    for (let i = 0; i < BLOCK_SIZE; i++) {
        const left = output[i * 2];
        const right = output[i * 2 + 1];
        const panGain = 0.5; // Linear pan law at center
        const ref = (0.5 * Math.sin(2 * Math.PI * 220 * i / SAMPLE_RATE)
                   + 0.5 * Math.sin(2 * Math.PI * 330 * i / SAMPLE_RATE)) * panGain;

        assert(Math.abs(left - right) < 1e-2, `L/R mismatch at ${i}: L=${left}, R=${right}`);
        assert(Math.abs(left - ref) < 1e-2, `sample ${i}: actual=${left}, ref=${ref}, diff=${Math.abs(left-ref)}`);
    }
    passed++;
    console.log('PASS: test_basic_sum_two_sines');
} catch (e) {
    console.error('FAIL: test_basic_sum_two_sines:', e.message);
    failed++;
}

// Test 2: Not silence (honesty gate)
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 2);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sine = sineWave(440, 1.0, BLOCK_SIZE);
    mixer.set_channel_input(0, sine);
    const output = mixer.process(BLOCK_SIZE);

    let max = 0;
    for (let i = 0; i < output.length; i++) max = Math.max(max, Math.abs(output[i]));
    assert(max > 0.01, `HONESTY GATE: near-silence max=${max}`);
    passed++;
    console.log('PASS: test_not_silence');
} catch (e) {
    console.error('FAIL: test_not_silence:', e.message);
    failed++;
}

// Test 3: Both channels present (honesty gate)
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sineA = sineWave(220, 0.5, BLOCK_SIZE);
    const zeros = new Float32Array(BLOCK_SIZE);

    mixer.set_channel_input(0, sineA);
    mixer.set_channel_input(1, zeros);

    let out1 = mixer.process(BLOCK_SIZE);
    let max1 = 0;
    for (let i = 0; i < out1.length; i++) max1 = Math.max(max1, Math.abs(out1[i]));

    // FIFO inputs: re-feed each block (worklet always re-feeds per block).
    mixer.set_channel_input(0, sineA);
    mixer.set_channel_input(1, sineA);
    let out2 = mixer.process(BLOCK_SIZE);
    let max2 = 0;
    for (let i = 0; i < out2.length; i++) max2 = Math.max(max2, Math.abs(out2[i]));

    assert(max2 > max1 * 1.5, `Both channels: max1=${max1}, max2=${max2}`);
    passed++;
    console.log('PASS: test_both_channels_present');
} catch (e) {
    console.error('FAIL: test_both_channels_present:', e.message);
    failed++;
}

// Test 4: Mute channel
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 2);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sine = sineWave(440, 0.5, BLOCK_SIZE);
    mixer.set_channel_input(0, sine);
    mixer.set_channel_mute(0, true);
    const output = mixer.process(BLOCK_SIZE);

    let max = 0;
    for (let i = 0; i < output.length; i++) max = Math.max(max, Math.abs(output[i]));
    assert(max < 1e-2, `Muted channel max=${max}`);
    passed++;
    console.log('PASS: test_mute_channel');
} catch (e) {
    console.error('FAIL: test_mute_channel:', e.message);
    failed++;
}

// Test 5: Gain control
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 2);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sine = sineWave(440, 1.0, BLOCK_SIZE);
    mixer.set_channel_input(0, sine);

    const outUnity = mixer.process(BLOCK_SIZE);

    mixer.set_channel_gain(0, 0.5);
    mixer.set_channel_input(0, sine); // FIFO: re-feed for the second block
    const outHalf = mixer.process(BLOCK_SIZE);

    for (let i = 0; i < BLOCK_SIZE; i++) {
        const unity = outUnity[i * 2];
        const half = outHalf[i * 2];
        if (Math.abs(unity) > 1e-4) {
            const ratio = half / unity;
            assert(Math.abs(ratio - 0.5) < 0.15, `Gain ratio at ${i}: expected ~0.5, got ${ratio}`);
        }
    }
    passed++;
    console.log('PASS: test_gain_control');
} catch (e) {
    console.error('FAIL: test_gain_control:', e.message);
    failed++;
}

// Helper: interleave two mono buffers into stereo (L,R,L,R,...)
function stereoInterleave(leftBuf, rightBuf) {
    const interleaved = new Float32Array(leftBuf.length * 2);
    for (let i = 0; i < leftBuf.length; i++) {
        interleaved[i * 2] = leftBuf[i];
        interleaved[i * 2 + 1] = rightBuf[i];
    }
    return interleaved;
}

// Test 6: Interleaved stereo input (known-answer)
// set_channel_input_interleaved de-interleaves into consecutive mixer channels.
// Two mono channels summed at Linear pan law center → 0.5 * (L + R).
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sineL = sineWave(220, 0.5, BLOCK_SIZE);
    const sineR = sineWave(330, 0.5, BLOCK_SIZE);
    const interleaved = stereoInterleave(sineL, sineR);

    mixer.set_channel_input_interleaved(0, interleaved, 2);
    const output = mixer.process(BLOCK_SIZE);
    assert(output.length === BLOCK_SIZE * 2, `output length ${output.length} !== ${BLOCK_SIZE * 2}`);

    const panGain = 0.5; // Linear pan law at center
    for (let i = 0; i < BLOCK_SIZE; i++) {
        const left = output[i * 2];
        const right = output[i * 2 + 1];
        const ref = (sineL[i] + sineR[i]) * panGain;
        assert(Math.abs(left - right) < 1e-2, `L/R mismatch at ${i}: L=${left}, R=${right}`);
        assert(Math.abs(left - ref) < 1e-2, `sample ${i}: actual=${left}, ref=${ref}, diff=${Math.abs(left - ref)}`);
    }
    passed++;
    console.log('PASS: test_interleaved_stereo_input');
} catch (e) {
    console.error('FAIL: test_interleaved_stereo_input:', e.message);
    failed++;
}

// Test 7: PID mapping with feed_pcm
// map_pid + feed_pcm routes a TS PID's audio through the mixer.
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sine = sineWave(440, 1.0, BLOCK_SIZE);
    const stereoData = stereoInterleave(sine, sine);

    mixer.map_pid(0x101, 0, 2);
    assert(mixer.pid_channel(0x101) === 0, `pid_channel should be 0, got ${mixer.pid_channel(0x101)}`);
    assert(mixer.pid_channel_count(0x101) === 2, `pid_channel_count should be 2, got ${mixer.pid_channel_count(0x101)}`);

    mixer.feed_pcm(0x101, stereoData);
    const output = mixer.process(BLOCK_SIZE);

    let max = 0;
    for (let i = 0; i < output.length; i++) max = Math.max(max, Math.abs(output[i]));
    assert(max > 0.01, `PID mapping output should be non-silent, max=${max}`);
    passed++;
    console.log('PASS: test_pid_mapping');
} catch (e) {
    console.error('FAIL: test_pid_mapping:', e.message);
    failed++;
}

// Test 8: Subscribe / unsubscribe lifecycle
// Audible → unsubscribe (silent) → subscribe (audible again).
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sine = sineWave(440, 1.0, BLOCK_SIZE);
    const stereoData = stereoInterleave(sine, sine);

    mixer.map_pid(0x101, 0, 2);

    // Subscribed (default) → audible
    mixer.feed_pcm(0x101, stereoData);
    let out1 = mixer.process(BLOCK_SIZE);
    let max1 = 0;
    for (let i = 0; i < out1.length; i++) max1 = Math.max(max1, Math.abs(out1[i]));
    assert(max1 > 0.01, `Subscribed PID should be audible, max1=${max1}`);

    // Unsubscribe → feed_pcm is dropped + channels muted → silent
    mixer.unsubscribe_pid(0x101);
    mixer.feed_pcm(0x101, stereoData);
    let out2 = mixer.process(BLOCK_SIZE);
    let max2 = 0;
    for (let i = 0; i < out2.length; i++) max2 = Math.max(max2, Math.abs(out2[i]));
    assert(max2 < 1e-2, `Unsubscribed PID should be silent, max2=${max2}`);

    // Re-subscribe → audible again
    mixer.subscribe_pid(0x101);
    mixer.feed_pcm(0x101, stereoData);
    let out3 = mixer.process(BLOCK_SIZE);
    let max3 = 0;
    for (let i = 0; i < out3.length; i++) max3 = Math.max(max3, Math.abs(out3[i]));
    assert(max3 > 0.01, `Re-subscribed PID should be audible, max3=${max3}`);
    passed++;
    console.log('PASS: test_subscribe_unsubscribe');
} catch (e) {
    console.error('FAIL: test_subscribe_unsubscribe:', e.message);
    failed++;
}

// Test 9: PID reconfiguration (unmap + remap)
// Mid-stream PID swap: unmap old PID, map new PID to same channels.
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const sine = sineWave(440, 1.0, BLOCK_SIZE);
    const stereoData = stereoInterleave(sine, sine);

    // First PID active
    mixer.map_pid(0x101, 0, 2);
    mixer.feed_pcm(0x101, stereoData);
    let out1 = mixer.process(BLOCK_SIZE);
    let max1 = 0;
    for (let i = 0; i < out1.length; i++) max1 = Math.max(max1, Math.abs(out1[i]));
    assert(max1 > 0.01, `First PID should produce audio, max1=${max1}`);

    // Reconfigure: unmap 0x101, map 0x102 to same channels
    mixer.unmap_pid(0x101);
    assert(mixer.pid_channel(0x101) === -1, `Unmapped PID should return -1, got ${mixer.pid_channel(0x101)}`);

    mixer.map_pid(0x102, 0, 2);
    assert(mixer.pid_channel(0x102) === 0, `New PID should map to channel 0, got ${mixer.pid_channel(0x102)}`);

    mixer.feed_pcm(0x102, stereoData);
    let out2 = mixer.process(BLOCK_SIZE);
    let max2 = 0;
    for (let i = 0; i < out2.length; i++) max2 = Math.max(max2, Math.abs(out2[i]));
    assert(max2 > 0.01, `Reconfigured PID should produce audio, max2=${max2}`);

    // Old PID feed should be silently ignored (unmapped); new PID must be
    // re-fed (FIFO inputs — the old block was consumed by out2's process).
    mixer.feed_pcm(0x101, stereoData);
    mixer.feed_pcm(0x102, stereoData);
    let out3 = mixer.process(BLOCK_SIZE);
    let max3 = 0;
    for (let i = 0; i < out3.length; i++) max3 = Math.max(max3, Math.abs(out3[i]));
    assert(max3 > 0.01, `New PID should still be active after old PID feed, max3=${max3}`);
    passed++;
    console.log('PASS: test_pid_reconfiguration');
} catch (e) {
    console.error('FAIL: test_pid_reconfiguration:', e.message);
    failed++;
}


// ── Dynamics: compressor affects output ───────────
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 8);
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true);
    mixer.set_eq_bypass(1, true);
    const input = sineWave(1000, 0.8, BLOCK_SIZE);
    mixer.set_channel_gain(0, 1.0);
    mixer.set_channel_input(0, input);
    const bypass = mixer.process(BLOCK_SIZE);

    mixer.enable_compressor(0);
    mixer.set_channel_input(0, input);
    const comp = mixer.process(BLOCK_SIZE);

    let diff = 0;
    for (let i = 0; i < bypass.length; i++) diff += Math.abs(bypass[i] - comp[i]);
    diff /= bypass.length;
    assert(diff > 0.001, 'compressor should change signal, mean diff=' + diff.toFixed(6));
    console.log('PASS: compressor affects output');
    passed++;
} catch (e) { console.error('FAIL: compressor affects output -', e.message); failed++; }

// ── Master limiter prevents clipping ───────────────
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 8);
    const input = sineWave(440, 1.0, BLOCK_SIZE);
    for (let ch = 0; ch < 8; ch++) mixer.set_channel_gain(ch, 1.0);

    let maxPeak = 0;
    for (let block = 0; block < 200; block++) {
        for (let ch = 0; ch < 8; ch++) mixer.set_channel_input(ch, input);
        const out = mixer.process(BLOCK_SIZE);
        for (let i = 0; i < out.length; i++) maxPeak = Math.max(maxPeak, Math.abs(out[i]));
    }
    assert(maxPeak < 1.0, 'limiter should prevent clipping, maxPeak=' + maxPeak.toFixed(4));
    console.log('PASS: master limiter prevents clipping');
    passed++;
} catch (e) { console.error('FAIL: master limiter prevents clipping -', e.message); failed++; }

// ── Unmapped PID counter ───────────────────────────
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 8);
    const cnt0 = Number(mixer.unmapped_pid_count());
    assert(cnt0 === 0, 'should start at 0, got ' + cnt0);
    const data = new Float32Array([0.5, 0.5]);
    mixer.feed_pcm(999, data);
    const cnt1 = Number(mixer.unmapped_pid_count());
    assert(cnt1 === 1, 'should increment, got ' + cnt1);
    console.log('PASS: unmapped PID counter works');
    passed++;
} catch (e) { console.error('FAIL: unmapped PID counter -', e.message); failed++; }


// ── Channel direct-out tap (post-chain post-fader mono, Nch) ──
// 4ch tap: ch0 sine unity, ch1 sine with fader 0.5, ch2/ch3 unfed → zeros.
// Tap is PRE pan: full input amplitude (master would apply the 0.5 Linear
// center-pan gain on top). take() drains — second take is empty.
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 4);
    mixer.set_limiter_enabled(false);
    for (let ch = 0; ch < 4; ch++) mixer.set_eq_bypass(ch, true);
    mixer.set_channel_tap(4);

    const sineA = sineWave(220, 0.5, BLOCK_SIZE);
    const sineB = sineWave(330, 0.25, BLOCK_SIZE);
    mixer.set_channel_input(0, sineA);
    mixer.set_channel_input(1, sineB);
    mixer.set_channel_gain(1, 0.5); // fader applies to the tap

    mixer.process(BLOCK_SIZE);
    const tap = mixer.take_channel_tap();
    assert(tap.length === BLOCK_SIZE * 4, `tap length ${tap.length} !== ${BLOCK_SIZE * 4}`);

    for (let i = 0; i < BLOCK_SIZE; i++) {
        assert(Math.abs(tap[i * 4] - sineA[i]) < 1e-4, `frame ${i} ch0: tap=${tap[i * 4]} ref=${sineA[i]}`);
        assert(Math.abs(tap[i * 4 + 1] - sineB[i] * 0.5) < 1e-4, `frame ${i} ch1: tap=${tap[i * 4 + 1]} ref=${sineB[i] * 0.5}`);
        assert(tap[i * 4 + 2] === 0, `frame ${i} ch2 must be silent, got ${tap[i * 4 + 2]}`);
        assert(tap[i * 4 + 3] === 0, `frame ${i} ch3 must be silent, got ${tap[i * 4 + 3]}`);
    }

    const tap2 = mixer.take_channel_tap();
    assert(tap2.length === 0, `tap must drain on take, got ${tap2.length} samples`);
    passed++;
    console.log('PASS: channel direct-out tap (Nch, post-fader pre-pan, zero-fill, drain)');
} catch (e) {
    console.error('FAIL: channel direct-out tap -', e.message);
    failed++;
}


// ── Multi-PID sum: N identical unity-gain PIDs → N× master level ──
// M2 known-answer. The claim is that the mixer sums N input PIDs
// linearly. Ratio-baseline approach: run the identical feeding scenario
// twice on separate mixer instances — one baseline PID vs N PIDs, all
// unity gain, same block count — and assert level_N ≈ N × level_1.
// Measuring the ratio (not an absolute level) makes the test immune to
// the pan law and any fixed master gain: they apply to both runs and
// cancel in the division.
// The master limiter is ALWAYS on (ceiling −0.3 dBFS ≈ 0.966 linear),
// so the sine gain is deliberately tiny (0.04): even 8 coherent copies
// at 0.5 Linear center-pan (8 × 0.04 × 0.5 = 0.16 peak) sit far below
// the ceiling — the limiter never engages and the sum stays linear.

// Helper: run the feed/process loop to steady state and return the RMS
// level of the master LEFT channel over the last MEASURE blocks. Every
// PID is fed `data` (interleaved for that PID's channel count) once per
// block — the FIFO inputs are consumed by each process(), so the worklet
// re-feed-every-block pattern applies. WARMUP blocks let the elastic
// FIFOs settle before measuring.
function multiPidSteadyStateRms(pidSpecs, data) {
    const WARMUP = 32;
    const MEASURE = 64;
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 8);
    for (let ch = 0; ch < 8; ch++) mixer.set_eq_bypass(ch, true);
    for (const [pid, ch, count] of pidSpecs) mixer.map_pid(pid, ch, count);
    let sumSq = 0;
    for (let block = 0; block < WARMUP + MEASURE; block++) {
        for (const [pid] of pidSpecs) mixer.feed_pcm(pid, data);
        const out = mixer.process(BLOCK_SIZE);
        if (block >= WARMUP) {
            for (let i = 0; i < BLOCK_SIZE; i++) sumSq += out[i * 2] * out[i * 2];
        }
    }
    return Math.sqrt(sumSq / (MEASURE * BLOCK_SIZE));
}

// Test: 8 mono PIDs (0x101..0x108 → channels 0..7), each fed the SAME
// sine buffer at unity gain → 8× the single-PID baseline level.
try {
    const sine = sineWave(440, 0.04, BLOCK_SIZE);

    const baseline = multiPidSteadyStateRms([[0x101, 0, 1]], sine);

    const pidSpecs = [];
    for (let i = 0; i < 8; i++) pidSpecs.push([0x101 + i, i, 1]);
    const summed = multiPidSteadyStateRms(pidSpecs, sine);

    const ratio = summed / baseline;
    assert(baseline > 1e-4, `baseline level suspiciously low: ${baseline}`);
    assert(Math.abs(ratio - 8) / 8 < 0.05,
        `8x mono sum: ratio=${ratio.toFixed(3)} (expected 8), baseline=${baseline.toFixed(5)}, summed=${summed.toFixed(5)}`);
    passed++;
    console.log('PASS: test_multi_pid_sum_8x_mono');
} catch (e) {
    console.error('FAIL: test_multi_pid_sum_8x_mono:', e.message);
    failed++;
}

// Test: 4 stereo PIDs (0x101..0x104 → channel pairs 0,2,4,6), each fed
// the SAME interleaved stereo sine (identical L/R) at unity gain → 4×
// the single-PID baseline level.
try {
    const sine = sineWave(440, 0.04, BLOCK_SIZE);
    const stereo = stereoInterleave(sine, sine); // identical L/R

    const baseline = multiPidSteadyStateRms([[0x101, 0, 2]], stereo);

    const pidSpecs = [];
    for (let i = 0; i < 4; i++) pidSpecs.push([0x101 + i, i * 2, 2]);
    const summed = multiPidSteadyStateRms(pidSpecs, stereo);

    const ratio = summed / baseline;
    assert(baseline > 1e-4, `baseline level suspiciously low: ${baseline}`);
    assert(Math.abs(ratio - 4) / 4 < 0.05,
        `4x stereo sum: ratio=${ratio.toFixed(3)} (expected 4), baseline=${baseline.toFixed(5)}, summed=${summed.toFixed(5)}`);
    passed++;
    console.log('PASS: test_multi_pid_sum_4x_stereo');
} catch (e) {
    console.error('FAIL: test_multi_pid_sum_4x_stereo:', e.message);
    failed++;
}


console.log(`\n=== Results: ${passed} passed, ${failed} failed ===`);
process.exit(failed > 0 ? 1 : 0);
