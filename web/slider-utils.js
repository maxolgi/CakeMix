// slider-utils.js — Professional audio slider/knob interactions
// Adds: logarithmic response, shift+drag fine-tune, double-click reset,
// wheel adjustment. Works on any <input type="range">.

// Convert fader position (0-1) to linear gain using dB scale
// Maps -∞ to +6 dB across the slider travel (like a real console fader)
export function faderToGain(pos) {
    // pos: 0.0 to 1.0 → gain: 0.0 to ~2.0 (linear)
    // dB range: -60 to +6
    const db = -60 + pos * 66;
    if (db <= -59) return 0;
    return Math.pow(10, db / 20);
}

// Convert linear gain back to fader position
export function gainToFader(gain) {
    if (gain <= 0.001) return 0;
    const db = 20 * Math.log10(gain);
    return Math.max(0, Math.min(1, (db + 60) / 66));
}

// Format gain for display
export function formatGain(gain) {
    if (gain <= 0.001) return "-\u221e";
    const db = 20 * Math.log10(gain);
    if (db >= 0) return "+" + db.toFixed(1);
    return db.toFixed(1);
}

// Format pan value
export function formatPan(pan) {
    if (Math.abs(pan) < 0.02) return "C";
    if (pan < 0) return "L" + Math.round(Math.abs(pan) * 100);
    return "R" + Math.round(pan * 100);
}

// Enhance a slider element with professional behaviors
export function enhanceSlider(el, options = {}) {
    const {
        onInput = null,
        onReset = null,
        defaultValue = null,
    } = options;

    let isDragging = false;
    let dragStartY = 0;
    let dragStartVal = 0;
    let isShift = false;

    // Double-click to reset
    el.addEventListener('dblclick', () => {
        const def = defaultValue !== null ? defaultValue : parseFloat(el.getAttribute('data-default') || el.value);
        el.value = def;
        el.dispatchEvent(new Event('input', { bubbles: true }));
        if (onReset) onReset(def);
    });

    // Wheel to fine-tune
    el.addEventListener('wheel', (e) => {
        e.preventDefault();
        const range = parseFloat(el.max) - parseFloat(el.min);
        const step = e.shiftKey ? range / 1000 : range / 200;
        const dir = e.deltaY < 0 ? 1 : -1;
        el.value = Math.max(parseFloat(el.min), Math.min(parseFloat(el.max), parseFloat(el.value) + dir * step));
        el.dispatchEvent(new Event('input', { bubbles: true }));
    }, { passive: false });

    // Track shift state for drag
    el.addEventListener('keydown', (e) => { if (e.key === 'Shift') isShift = true; });
    el.addEventListener('keyup', (e) => { if (e.key === 'Shift') isShift = false; });
}

// Format dB for EQ display
export function formatDb(db) {
    if (db > 0) return "+" + db.toFixed(1);
    return db.toFixed(1);
}
