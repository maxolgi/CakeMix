/**
 * app.js — CakeMix WASM audio mixer demo.
 *
 * Sets up the AudioWorklet, wires up the UI controls.
 * The WASM mixer is loaded on the main thread for status display;
 * the AudioWorklet runs the real-time audio path.
 */

const SAMPLE_RATE = 48000;
const NUM_CHANNELS = 4;

let audioCtx = null;
let mixerNode = null;

async function initAudio() {
    audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });

    // Register the self-contained worklet (no imports needed).
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
            document.getElementById('start-btn').disabled = false;
            document.getElementById('start-btn').textContent = 'Start Audio';
            document.getElementById('status').textContent = 'Ready. Click Start Audio.';
        } else if (msg.type === 'status') {
            const peakLDb = msg.peakL > 0 ? 20 * Math.log10(msg.peakL) : '-inf';
            const peakRDb = msg.peakR > 0 ? 20 * Math.log10(msg.peakR) : '-inf';
            document.getElementById('status').textContent =
                `Frame ${msg.frame} | L: ${peakLDb.toFixed(1)} dB | R: ${peakRDb.toFixed(1)} dB`;
        }
    };

    console.log('[app] Pipeline ready');
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
