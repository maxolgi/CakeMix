const SAMPLE_RATE = 48000;
const NUM_CHANNELS = 4;

let audioCtx = null;
let mixerNode = null;
let mode = "demo"; // "demo" or "live"

async function initAudio() {
    try {
        audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
        await audioCtx.audioWorklet.addModule("/mixer-worklet-processor.js");
        mixerNode = new AudioWorkletNode(audioCtx, "mixer-processor", {
            numberOfInputs: 0,
            numberOfOutputs: 1,
            outputChannelCount: [2],
        });
        mixerNode.connect(audioCtx.destination);

        mixerNode.port.onmessage = (e) => {
            const msg = e.data;
            if (msg.type === "ready") {
                loadAndSendWasm();
            } else if (msg.type === "wasm-ready") {
                document.getElementById("start-btn").disabled = false;
                document.getElementById("start-btn").textContent = "Start Audio";
                document.getElementById("status").textContent = "WASM mixer ready.";
                document.getElementById("wasm-badge").textContent = "WASM ACTIVE";
                document.getElementById("wasm-badge").classList.add("active");
            } else if (msg.type === "error") {
                document.getElementById("status").textContent = "Error: " + msg.msg;
                document.getElementById("start-btn").disabled = false;
                document.getElementById("start-btn").textContent = "Retry";
            } else if (msg.type === "meter") {
                updateMeters(msg);
            }
        };
    } catch(e) {
        console.error("initAudio failed:", e);
        document.getElementById("status").textContent = "Init failed: " + e.message;
    }
}

async function loadAndSendWasm() {
    try {
        const wasmResponse = await fetch("/pkg/mixer_wasm_bg.wasm");
        const wasmBytes = await wasmResponse.arrayBuffer();
        mixerNode.port.postMessage({ type: "init-wasm", wasmBytes: wasmBytes });
    } catch (err) {
        console.error("WASM load failed:", err);
        document.getElementById("status").textContent = "WASM load failed";
    }
}

function sendToWorklet(msg) {
    if (mixerNode) mixerNode.port.postMessage(msg);
}

function updateMeters(msg) {
    const formatDb = (db) => db === -Infinity || db < -60 ? "-∞" : db.toFixed(1);
    const peakL = msg.peakL, peakR = msg.peakR;
    const rmsL = msg.rmsL, rmsR = msg.rmsR;

    // Update meter bars (scale -60 to 0 dB → 0% to 100%)
    const pct = (db) => Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
    const lBar = document.getElementById("meter-l");
    const rBar = document.getElementById("meter-r");
    if (lBar) lBar.style.width = pct(peakL) + "%";
    if (rBar) rBar.style.width = pct(peakR) + "%";

    // Update text
    const meterText = document.getElementById("meter-text");
    if (meterText) {
        meterText.textContent = `L: ${formatDb(rmsL)} / ${formatDb(peakL)} dB  |  R: ${formatDb(rmsR)} / ${formatDb(peakR)} dB`;
    }

    // Clip indicator
    if (msg.clip) {
        const clipEl = document.getElementById("clip-indicator");
        if (clipEl) clipEl.classList.add("active");
    }
}

// ── WebSRT Integration ──────────────────────────────────────────
// When in live mode, the WebSRT worker posts {type:'pcm'} messages.
// We relay them to the mixer worklet here.
// In a full integration, this would come from WebSRT's worker via
// window.postMessage or a shared MessagePort.
function relayPcmData(pid, samples, channelCount) {
    sendToWorklet({ type: "pcm", pid: pid, samples: samples });
}

// Handle PID map updates from WebSRT (PMT events)
function handlePidMap(streams) {
    for (const s of streams) {
        sendToWorklet({ type: "map-pid", pid: s.pid, chStart: s.chStart, channelCount: s.channelCount });
    }
}

function setupUI() {
    document.getElementById("start-btn").addEventListener("click", async () => {
        if (audioCtx && audioCtx.state === "suspended") await audioCtx.resume();
        sendToWorklet({ type: "start" });
        document.getElementById("start-btn").disabled = true;
        document.getElementById("start-btn").textContent = "Running";
        document.getElementById("stop-btn").disabled = false;
    });

    const stopBtn = document.getElementById("stop-btn");
    if (stopBtn) {
        stopBtn.addEventListener("click", () => {
            sendToWorklet({ type: "stop" });
            document.getElementById("start-btn").disabled = false;
            document.getElementById("start-btn").textContent = "Resume";
            stopBtn.disabled = true;
            const clipEl = document.getElementById("clip-indicator");
            if (clipEl) clipEl.classList.remove("active");
        });
    }

    // Channel controls
    for (let ch = 0; ch < NUM_CHANNELS; ch++) {
        const gainEl = document.getElementById("ch" + ch + "-gain");
        const panEl = document.getElementById("ch" + ch + "-pan");
        const muteEl = document.getElementById("ch" + ch + "-mute");

        if (gainEl) gainEl.addEventListener("input", () => {
            sendToWorklet({ type: "set-gain", ch, gain: parseFloat(gainEl.value) });
            const valEl = document.getElementById("ch" + ch + "-gain-val");
            if (valEl) valEl.textContent = parseFloat(gainEl.value).toFixed(2);
        });
        if (panEl) panEl.addEventListener("input", () => {
            sendToWorklet({ type: "set-pan", ch, pan: parseFloat(panEl.value) });
            const valEl = document.getElementById("ch" + ch + "-pan-val");
            if (valEl) valEl.textContent = parseFloat(panEl.value).toFixed(2);
        });
        if (muteEl) muteEl.addEventListener("click", () => {
            const muted = muteEl.classList.toggle("active");
            sendToWorklet({ type: "set-mute", ch, muted });
            muteEl.textContent = muted ? "Muted" : "Mute";
        });
    }
}

setupUI();
initAudio();
