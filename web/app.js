/**
 * app.js — CakeMix WASM audio mixer demo.
 *
 * Loads the WASM mixer, creates an AudioContext, registers the worklet,
 * sends the compiled wasm Module to the worklet, and wires up UI controls.
 *
 * The mixer runs IN the AudioWorklet for real-time audio.
 * Test signal generators provide input (no WebSRT needed).
 */

const SAMPLE_RATE = 48000;
const NUM_CHANNELS = 4;

let audioCtx = null;
let mixerNode = null;

async function initAudio() {
    audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });

    // Register the worklet processor.
    await audioCtx.audioWorklet.addModule('/mixer-worklet-processor.js');

    mixerNode = new AudioWorkletNode(audioCtx, 'mixer-processor', {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
    });

    mixerNode.connect(audioCtx.destination);

    mixerNode.port.onmessage = (e) => {
        const msg = e.data;
        if (msg.type === 'ready') {
            // Worklet is ready — now load WASM and send the compiled module.
            loadAndSendWasm();
        } else if (msg.type === 'wasm-ready') {
            document.getElementById('start-btn').disabled = false;
            document.getElementById('start-btn').textContent = 'Start Audio';
            document.getElementById('status').textContent = 'WASM mixer ready. Click Start Audio.';
            document.getElementById('wasm-badge').textContent = 'WASM ACTIVE';
            document.getElementById('wasm-badge').classList.add('active');
        } else if (msg.type === 'error') {
            document.getElementById('status').textContent = 'Error: ' + msg.msg;
            // Fall back to JS mixer (still works, just without WASM DSP).
            document.getElementById('start-btn').disabled = false;
            document.getElementById('start-btn').textContent = 'Start Audio (JS fallback)';
        } else if (msg.type === 'status') {
            const peakLDb = msg.peakL > 0 ? 20 * Math.log10(msg.peakL) : '-inf';
            const peakRDb = msg.peakR > 0 ? 20 * Math.log10(msg.peakR) : '-inf';
            const engine = msg.wasmActive ? 'WASM' : 'JS';
            document.getElementById('status').textContent =
                `[${engine}] Frame ${msg.frame} | L: ${peakLDb.toFixed(1)} dB | R: ${peakRDb.toFixed(1)} dB`;
        }
    };
}

async function loadAndSendWasm() {
    try {
        // Fetch and compile the wasm module on the main thread.
        const wasmResponse = await fetch('/pkg/mixer_wasm_bg.wasm');
        const wasmBytes = await wasmResponse.arrayBuffer();
        const wasmModule = await WebAssembly.compile(wasmBytes);

        // Send the compiled module to the worklet.
        // WebAssembly.Module is structured-cloneable across MessagePort.
        mixerNode.port.postMessage({ type: 'init-wasm', module: wasmModule });
    } catch (err) {
        console.error('[app] WASM load failed:', err);
        document.getElementById('status').textContent = 'WASM load failed — using JS fallback';
        document.getElementById('start-btn').disabled = false;
        document.getElementById('start-btn').textContent = 'Start Audio (JS fallback)';
    }
}

function sendToWorklet(msg) {
    if (mixerNode) mixerNode.port.postMessage(msg);
}

function setupUI() {
    document.getElementById('start-btn').addEventListener('click', async () => {
        if (audioCtx.state === 'suspended') await audioCtx.resume();
        sendToWorklet({ type: 'start' });
        document.getElementById('start-btn').disabled = true;
        document.getElementById('start-btn').textContent = 'Running';
        document.getElementById('stop-btn').disabled = false;
    });

    const stopBtn = document.getElementById('stop-btn');
    if (stopBtn) {
        stopBtn.addEventListener('click', () => {
            sendToWorklet({ type: 'stop' });
            document.getElementById('start-btn').disabled = false;
            document.getElementById('start-btn').textContent = 'Resume';
            stopBtn.disabled = true;
        });
    }

    for (let ch = 0; ch < NUM_CHANNELS; ch++) {
        const gainEl = document.getElementById(`ch${ch}-gain`);
        const panEl = document.getElementById(`ch${ch}-pan`);
        const muteEl = document.getElementById(`ch${ch}-mute`);

        gainEl?.addEventListener('input', () => {
            sendToWorklet({ type: 'set-gain', ch, gain: parseFloat(gainEl.value) });
            document.getElementById(`ch${ch}-gain-val`).textContent = parseFloat(gainEl.value).toFixed(2);
        });
        panEl?.addEventListener('input', () => {
            sendToWorklet({ type: 'set-pan', ch, pan: parseFloat(panEl.value) });
            document.getElementById(`ch${ch}-pan-val`).textContent = parseFloat(panEl.value).toFixed(2);
        });
        muteEl?.addEventListener('click', () => {
            const muted = muteEl.classList.toggle('active');
            sendToWorklet({ type: 'set-mute', ch, muted });
            muteEl.textContent = muted ? 'Muted' : 'Mute';
        });
    }
}

setupUI();
initAudio().catch(err => {
    console.error('[app] Init failed:', err);
    document.getElementById('status').textContent = 'Failed: ' + err.message;
});
