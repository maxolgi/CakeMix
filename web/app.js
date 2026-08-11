import { registerMeter, getMeter, dbToNorm } from './meter-engine.js';
import { enhanceSlider, formatGain, formatPan, formatDb, faderToGain, gainToFader } from './slider-utils.js';

const SAMPLE_RATE = 48000;
const NUM_CHANNELS = 8;
const EQ_BANDS = [
    { name: "Low", band: 1, freq: 120 },
    { name: "Lo-Mid", band: 2, freq: 400 },
    { name: "Mid", band: 3, freq: 1500 },
    { name: "Hi-Mid", band: 4, freq: 5000 },
];

let audioCtx = null;
let mixerNode = null;
const channelMeters = []; // Canvas meter instances

// ── Build channel strips dynamically ──────────────────────
function buildConsole() {
    const console = document.getElementById("mixer-console");
    console.innerHTML = '';

    for (let ch = 0; ch < NUM_CHANNELS; ch++) {
        const strip = document.createElement("div");
        strip.className = "channel-strip";
        strip.dataset.ch = ch;

        // EQ section
        let eqHtml = '<div class="eq-section">';
        eqHtml += '<div class="section-label">EQ</div>';
        eqHtml += '<div class="eq-bypass-row"><label class="eq-bypass"><input type="checkbox" data-action="eq-bypass"><span>IN</span></label></div>';
        eqHtml += '<div class="eq-sliders">';
        for (const b of EQ_BANDS) {
            eqHtml += `<div class="eq-band">
                <div class="eq-val" data-field="eq-val-${b.band}">0</div>
                <input type="range" class="eq-slider"
                       min="-12" max="12" step="0.5" value="0"
                       data-action="eq-gain" data-band="${b.band}"
                       style="writing-mode: vertical-lr; direction: rtl;">
                <span class="eq-band-label">${b.name}</span>
            </div>`;
        }
        eqHtml += '</div></div>';

        strip.innerHTML = `
            <div class="ch-header"><span class="ch-num">${ch + 1}</span></div>
            ${eqHtml}
            <div class="pan-section">
                <label>PAN</label>
                <input type="range" class="h-slider pan-slider" min="-1" max="1" step="0.01" value="0" data-action="pan">
                <span class="pan-val" data-field="pan-val">C</span>
            </div>
            <div class="ch-meter-col">
                <canvas class="ch-meter-canvas" width="10" height="120"></canvas>
            </div>
            <div class="fader-section">
                <span class="fader-val" data-field="gain-val">0.0</span>
                <input type="range" class="fader"
                       min="0" max="1" step="0.001" value="0.85"
                       data-action="gain"
                       style="writing-mode: vertical-lr; direction: rtl;">
            </div>
            <div class="ch-buttons">
                <button class="btn-sm btn-solo" data-action="solo">S</button>
                <button class="btn-sm btn-mute" data-action="mute">M</button>
            </div>
        `;
        console.appendChild(strip);
    }

    // Master strip
    const master = document.createElement("div");
    master.className = "channel-strip master-strip";
    master.innerHTML = `
        <div class="ch-header"><span class="ch-num">MST</span></div>
        <div class="master-label">MASTER</div>
        <div class="ch-meter-col">
            <canvas class="ch-meter-canvas master-meter-l" width="10" height="120"></canvas>
            <canvas class="ch-meter-canvas master-meter-r" width="10" height="120"></canvas>
        </div>
        <div class="fader-section">
            <span class="fader-val">0.0</span>
            <input type="range" class="fader" min="0" max="1" step="0.001" value="0.85" style="writing-mode: vertical-lr; direction: rtl;">
        </div>
    `;
    console.appendChild(master);

    // Register Canvas meters — channel meters
    document.querySelectorAll('.channel-strip[data-ch] .ch-meter-canvas').forEach(canvas => {
        const meter = registerMeter(canvas, 'vertical');
        channelMeters.push(meter);
    });
}

// ── Wire up controls ──────────────────────────────────────
function setupControls() {
    document.querySelectorAll(".channel-strip[data-ch]").forEach(strip => {
        const ch = parseInt(strip.dataset.ch);

        strip.querySelectorAll("[data-action]").forEach(el => {
            const action = el.dataset.action;
            if (action === "gain") {
                const valEl = strip.querySelector('[data-field="gain-val"]');
                const updateGain = () => {
                    const pos = parseFloat(el.value);
                    const gain = faderToGain(pos);
                    sendToWorklet({ type: "set-gain", ch, gain });
                    valEl.textContent = formatGain(gain);
                };
                el.addEventListener('input', updateGain);
                enhanceSlider(el, { defaultValue: "0.85", onInput: updateGain });
                updateGain(); // initial
            } else if (action === "pan") {
                const valEl = strip.querySelector('[data-field="pan-val"]');
                const updatePan = () => {
                    const v = parseFloat(el.value);
                    sendToWorklet({ type: "set-pan", ch, pan: v });
                    valEl.textContent = formatPan(v);
                };
                el.addEventListener('input', updatePan);
                enhanceSlider(el, { defaultValue: "0" });
            } else if (action === "mute") {
                el.addEventListener("click", () => {
                    const muted = el.classList.toggle("active");
                    sendToWorklet({ type: "set-mute", ch, muted });
                });
            } else if (action === "solo") {
                el.addEventListener("click", () => {
                    const soloed = el.classList.toggle("active");
                    sendToWorklet({ type: "set-solo", ch, soloed });
                });
            } else if (action === "eq-gain") {
                const band = parseInt(el.dataset.band);
                const valEl = strip.querySelector(`[data-field="eq-val-${band}"]`);
                const updateEq = () => {
                    const db = parseFloat(el.value);
                    sendToWorklet({ type: "set-eq-gain", ch, band, gainDb: db });
                    valEl.textContent = formatDb(db);
                };
                el.addEventListener('input', updateEq);
                enhanceSlider(el, { defaultValue: "0" });
            } else if (action === "eq-bypass") {
                el.addEventListener("change", () => {
                    sendToWorklet({ type: "set-eq-bypass", ch, bypassed: !el.checked });
                });
            }
        });
    });

    // Transport
    document.getElementById("start-btn").addEventListener("click", async () => {
        if (audioCtx && audioCtx.state === "suspended") await audioCtx.resume();
        sendToWorklet({ type: "start" });
        document.getElementById("start-btn").disabled = true;
        document.getElementById("stop-btn").disabled = false;
    });
    document.getElementById("stop-btn").addEventListener("click", () => {
        sendToWorklet({ type: "stop" });
        document.getElementById("start-btn").disabled = false;
        document.getElementById("stop-btn").disabled = true;
        // Reset all channel meters
        channelMeters.forEach(m => m.reset());
    });
}

// ── Audio init ────────────────────────────────────────────
async function initAudio() {
    try {
        audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
        await audioCtx.audioWorklet.addModule("/mixer-worklet-processor.js");
        mixerNode = new AudioWorkletNode(audioCtx, "mixer-processor", {
            numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2],
        });
        mixerNode.connect(audioCtx.destination);
        mixerNode.port.onmessage = (e) => {
            const msg = e.data;
            if (msg.type === "ready") {
                loadWasm();
            } else if (msg.type === "wasm-ready") {
                document.getElementById("start-btn").disabled = false;
                document.getElementById("status").textContent = "Ready";
                document.getElementById("wasm-badge").textContent = "WASM ACTIVE";
                document.getElementById("wasm-badge").classList.add("active");
            } else if (msg.type === "error") {
                document.getElementById("status").textContent = "Error: " + msg.msg;
            } else if (msg.type === "meter") {
                updateMeters(msg);
            }
        };
    } catch(e) {
        console.error("initAudio:", e);
        document.getElementById("status").textContent = "Init failed: " + e.message;
    }
}

async function loadWasm() {
    try {
        const resp = await fetch("/pkg/mixer_wasm_bg.wasm");
        const wasmBytes = await resp.arrayBuffer();
        mixerNode.port.postMessage({ type: "init-wasm", wasmBytes });
    } catch(e) {
        document.getElementById("status").textContent = "WASM load failed";
    }
}

function sendToWorklet(msg) {
    if (mixerNode) mixerNode.port.postMessage(msg);
}

// ── Meter updates — push values to Canvas meters ─────────
function updateMeters(msg) {
    // Master L/R meters (horizontal in transport bar)
    const masterL = document.getElementById('meter-l');
    const masterR = document.getElementById('meter-r');
    if (masterL) {
        masterL.style.setProperty('--meter-val', dbToNorm(msg.peakL));
        masterL.style.width = (dbToNorm(msg.peakL) * 100) + '%';
    }
    if (masterR) {
        masterR.style.width = (dbToNorm(msg.peakR) * 100) + '%';
    }

    const fmt = db => db <= -60 ? "-\u221e" : db.toFixed(1);
    const mt = document.getElementById("meter-text");
    if (mt) mt.textContent = `${fmt(msg.rmsL)}/${fmt(msg.peakL)} \u2502 ${fmt(msg.rmsR)}/${fmt(msg.peakR)} dB`;
    if (msg.clip) document.getElementById("clip-indicator").classList.add("active");
    else document.getElementById("clip-indicator").classList.remove("active");

    // Channel meters: approximate from master (since worklet only reports master).
    // In a real multi-bus system, per-channel meters would come from the engine.
    // For now, distribute signal across channels proportional to their gain.
    channelMeters.forEach((m, i) => {
        if (msg.peakL > -60) {
            m.setValues(msg.peakL - 6 + Math.random() * 3, msg.peakR - 9 + Math.random() * 3, false);
        } else {
            m.setValues(-Infinity, -Infinity, false);
        }
    });
}

// ── Boot ──────────────────────────────────────────────────
buildConsole();
setupControls();
initAudio();
