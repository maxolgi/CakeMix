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

        assert(Math.abs(left - right) < 1e-6, `L/R mismatch at ${i}: L=${left}, R=${right}`);
        assert(Math.abs(left - ref) < 1e-5, `sample ${i}: actual=${left}, ref=${ref}, diff=${Math.abs(left-ref)}`);
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
    const sineA = sineWave(220, 0.5, BLOCK_SIZE);
    const zeros = new Float32Array(BLOCK_SIZE);

    mixer.set_channel_input(0, sineA);
    mixer.set_channel_input(1, zeros);

    let out1 = mixer.process(BLOCK_SIZE);
    let max1 = 0;
    for (let i = 0; i < out1.length; i++) max1 = Math.max(max1, Math.abs(out1[i]));

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
    const sine = sineWave(440, 0.5, BLOCK_SIZE);
    mixer.set_channel_input(0, sine);
    mixer.set_channel_mute(0, true);
    const output = mixer.process(BLOCK_SIZE);

    let max = 0;
    for (let i = 0; i < output.length; i++) max = Math.max(max, Math.abs(output[i]));
    assert(max < 1e-6, `Muted channel max=${max}`);
    passed++;
    console.log('PASS: test_mute_channel');
} catch (e) {
    console.error('FAIL: test_mute_channel:', e.message);
    failed++;
}

// Test 5: Gain control
try {
    const mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 2);
    const sine = sineWave(440, 1.0, BLOCK_SIZE);
    mixer.set_channel_input(0, sine);

    const outUnity = mixer.process(BLOCK_SIZE);

    mixer.set_channel_gain(0, 0.5);
    const outHalf = mixer.process(BLOCK_SIZE);

    for (let i = 0; i < BLOCK_SIZE; i++) {
        const unity = outUnity[i * 2];
        const half = outHalf[i * 2];
        if (Math.abs(unity) > 1e-4) {
            const ratio = half / unity;
            assert(Math.abs(ratio - 0.5) < 0.01, `Gain ratio at ${i}: expected ~0.5, got ${ratio}`);
        }
    }
    passed++;
    console.log('PASS: test_gain_control');
} catch (e) {
    console.error('FAIL: test_gain_control:', e.message);
    failed++;
}

console.log(`\n=== Results: ${passed} passed, ${failed} failed ===`);
process.exit(failed > 0 ? 1 : 0);
