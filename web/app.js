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

// ── Build channel strips dynamically ──────────────────────
function buildConsole() {
    const console = document.getElementById("mixer-console");
    for (let ch = 0; ch < NUM_CHANNELS; ch++) {
        const strip = document.createElement("div");
        strip.className = "channel-strip";
        strip.dataset.ch = ch;

        // EQ section
        let eqHtml = '<div class="eq-section">';
        eqHtml += '<div class="section-label">EQ</div>';
        eqHtml += '<div class="eq-bypass-row"><label class="eq-bypass"><input type="checkbox" data-action="eq-bypass"><span>EQ</span></label></div>';
        eqHtml += '<div class="eq-sliders">';
        for (const b of EQ_BANDS) {
            eqHtml += `
                <div class="eq-band">
                    <input type="range" class="v-slider eq-slider"
                           min="-12" max="12" step="0.5" value="0"
                           data-action="eq-gain" data-band="${b.band}"
                           orient="vertical">
                    <span class="eq-band-label">${b.name}</span>
                </div>`;
        }
        eqHtml += '</div></div>';

        strip.innerHTML = `
            <div class="ch-header">
                <span class="ch-num">${ch + 1}</span>
            </div>
            ${eqHtml}
            <div class="pan-section">
                <label>PAN</label>
                <input type="range" class="h-slider pan-slider" min="-1" max="1" step="0.01" value="${ch % 2 === 0 ? 0 : 0}" data-action="pan">
                <span class="pan-val" data-field="pan-val">C</span>
            </div>
            <div class="ch-meter-col">
                <div class="v-meter-track"><div class="v-meter-bar" data-field="ch-meter"></div></div>
            </div>
            <div class="fader-section">
                <input type="range" class="v-slider fader"
                       min="0" max="1.5" step="0.01" value="1.0"
                       data-action="gain" orient="vertical">
                <span class="fader-val" data-field="gain-val">1.00</span>
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
        <div class="ch-header"><span class="ch-num">M</span></div>
        <div class="master-label">MASTER</div>
        <div class="ch-meter-col">
            <div class="v-meter-track"><div class="v-meter-bar master-meter-l"></div></div>
            <div class="v-meter-track"><div class="v-meter-bar master-meter-r"></div></div>
        </div>
        <div class="fader-section">
            <input type="range" class="v-slider fader" min="0" max="1.5" step="0.01" value="1.0" orient="vertical">
            <span class="fader-val">1.00</span>
        </div>
    `;
    console.appendChild(master);
}

// ── Wire up controls ──────────────────────────────────────
function setupControls() {
    document.querySelectorAll(".channel-strip[data-ch]").forEach(strip => {
        const ch = parseInt(strip.dataset.ch);

        strip.querySelectorAll("[data-action]").forEach(el => {
            const action = el.dataset.action;
            if (action === "gain") {
                el.addEventListener("input", () => {
                    const v = parseFloat(el.value);
                    sendToWorklet({ type: "set-gain", ch, gain: v });
                    strip.querySelector('[data-field="gain-val"]').textContent = v.toFixed(2);
                });
            } else if (action === "pan") {
                el.addEventListener("input", () => {
                    const v = parseFloat(el.value);
                    sendToWorklet({ type: "set-pan", ch, pan: v });
                    const lbl = strip.querySelector('[data-field="pan-val"]');
                    if (Math.abs(v) < 0.05) lbl.textContent = "C";
                    else lbl.textContent = v < 0 ? `L${Math.round(-v*100)}` : `R${Math.round(v*100)}`;
                });
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
                el.addEventListener("input", () => {
                    const band = parseInt(el.dataset.band);
                    const db = parseFloat(el.value);
                    sendToWorklet({ type: "set-eq-gain", ch, band, gainDb: db });
                });
            } else if (action === "eq-bypass") {
                el.addEventListener("change", () => {
                    sendToWorklet({ type: "set-eq-bypass", ch, bypassed: el.checked });
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

// ── Meter updates ─────────────────────────────────────────
function updateMeters(msg) {
    const fmt = db => db <= -60 ? "-\u221e" : db.toFixed(1);
    const pct = db => Math.max(0, Math.min(100, ((db + 60) / 60) * 100));

    // Master meters
    const ml = document.getElementById("meter-l");
    const mr = document.getElementById("meter-r");
    if (ml) ml.style.width = pct(msg.peakL) + "%";
    if (mr) mr.style.width = pct(msg.peakR) + "%";
    const mt = document.getElementById("meter-text");
    if (mt) mt.textContent = `${fmt(msg.rmsL)}/${fmt(msg.peakL)} | ${fmt(msg.rmsR)}/${fmt(msg.peakR)} dB`;
    if (msg.clip) document.getElementById("clip-indicator").classList.add("active");
}

// ── Boot ──────────────────────────────────────────────────
buildConsole();
setupControls();
initAudio();
