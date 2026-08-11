/**
 * app.js — CakeMix WASM mixer web demo.
 *
 * Loads the WASM mixer, creates an AudioContext, registers the
 * AudioWorklet processor, and wires up the UI controls.
 */

import init from '../crates/mixer-wasm/pkg/mixer_wasm.js';

const SAMPLE_RATE = 48000;

let audioCtx = null;
let mixerNode = null;

async function initAudio() {
    // Load and compile the WASM module (needed to register imports).
    await init();
    console.log('[app] WASM loaded');

    audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });

    // Compile a separate copy for the worklet.
    const wasmResponse = await fetch('../crates/mixer-wasm/pkg/mixer_wasm_bg.wasm');
    const wasmBytes = await wasmResponse.arrayBuffer();
    const wasmModule = await WebAssembly.compile(wasmBytes);

    // Register the worklet processor.
    await audioCtx.audioWorklet.addModule('./mixer-worklet-processor.js');
    console.log('[app] Worklet loaded');

    mixerNode = new AudioWorkletNode(audioCtx, 'mixer-processor', {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
        processorOptions: { module: wasmModule },
    });

    mixerNode.connect(audioCtx.destination);
    mixerNode.port.onmessage = (e) => {
        if (e.data.type === 'ready') {
            document.getElementById('start-btn').disabled = false;
            document.getElementById('start-btn').textContent = 'Start Audio';
            document.getElementById('status').textContent = 'Ready.';
        } else if (e.data.type === 'status') {
            document.getElementById('status').textContent =
                `Processing (frame ${e.data.frame}, sample=${e.data.sample.toFixed(4)})`;
        }
    };

    console.log('[app] Pipeline ready');
}

function setupUI() {
    document.getElementById('start-btn').addEventListener('click', async () => {
        if (audioCtx.state === 'suspended') await audioCtx.resume();
        document.getElementById('start-btn').disabled = true;
        document.getElementById('start-btn').textContent = 'Running';
    });

    const FREQS = [220, 277.18, 329.63, 440];
    for (let ch = 0; ch < FREQS.length; ch++) {
        const gainEl = document.getElementById(`ch${ch}-gain`);
        const panEl = document.getElementById(`ch${ch}-pan`);
        const muteEl = document.getElementById(`ch${ch}-mute`);

        gainEl?.addEventListener('input', () => {
            mixerNode?.port.postMessage({ type: 'set-gain', ch, gain: parseFloat(gainEl.value) });
            document.getElementById(`ch${ch}-gain-val`).textContent = parseFloat(gainEl.value).toFixed(2);
        });
        panEl?.addEventListener('input', () => {
            mixerNode?.port.postMessage({ type: 'set-pan', ch, pan: parseFloat(panEl.value) });
            document.getElementById(`ch${ch}-pan-val`).textContent = parseFloat(panEl.value).toFixed(2);
        });
        muteEl?.addEventListener('click', () => {
            const muted = muteEl.classList.toggle('active');
            mixerNode?.port.postMessage({ type: 'set-mute', ch, muted });
        });
    }
}

setupUI();
initAudio().catch(err => {
    console.error('[app] Init failed:', err);
    document.getElementById('status').textContent = 'Failed: ' + err.message;
});
