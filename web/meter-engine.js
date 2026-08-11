// meter-engine.js — Canvas-based audio meter rendering
// Zero DOM reflow. Uses requestAnimationFrame. Supports peak hold + decay.

const METER_COLORS = {
    green: '#22c55e',
    yellow: '#eab308',
    red: '#ef4444',
    bg: 'rgba(0, 0, 0, 0.6)',
    bgTrack: 'rgba(255, 255, 255, 0.04)',
};

// dB → normalized position (0 = -60 dB, 1 = 0 dB, clamped)
function dbToNorm(db) {
    if (db <= -60) return 0;
    if (db >= 0) return 1;
    return (db + 60) / 60;
}

class MeterBar {
    constructor(canvas, orientation = 'vertical') {
        this.canvas = canvas;
        this.ctx = canvas.getContext('2d');
        this.orientation = orientation;
        // Current display values (smoothed)
        this.displayPeak = 0;
        this.displayRms = 0;
        // Target values (set from worklet messages)
        this.targetPeak = -Infinity;
        this.targetRms = -Infinity;
        // Peak hold marker
        this.peakHold = -Infinity;
        this.peakHoldTimer = 0;
        // Clip
        this.clipped = false;
        // Setup HiDPI
        this.setupDPI();
    }

    setupDPI() {
        const dpr = window.devicePixelRatio || 1;
        const rect = this.canvas.getBoundingClientRect();
        this.canvas.width = rect.width * dpr;
        this.canvas.height = rect.height * dpr;
        this.ctx.scale(dpr, dpr);
        this.w = rect.width;
        this.h = rect.height;
    }

    setValues(peakDb, rmsDb, clip) {
        this.targetPeak = peakDb;
        this.targetRms = rmsDb;
        if (clip) {
            this.clipped = true;
            setTimeout(() => { this.clipped = false; }, 1000);
        }
    }

    reset() {
        this.targetPeak = -Infinity;
        this.targetRms = -Infinity;
        this.peakHold = -Infinity;
        this.displayPeak = 0;
        this.displayRms = 0;
    }

    update() {
        const tPeak = dbToNorm(this.targetPeak);
        const tRms = dbToNorm(this.targetRms);

        // Attack: fast rise (0.3 of the gap per frame)
        // Release: slow fall (0.08 of the gap per frame)
        if (tPeak > this.displayPeak) {
            this.displayPeak += (tPeak - this.displayPeak) * 0.3;
        } else {
            this.displayPeak += (tPeak - this.displayPeak) * 0.08;
        }
        if (tRms > this.displayRms) {
            this.displayRms += (tRms - this.displayRms) * 0.3;
        } else {
            this.displayRms += (tRms - this.displayRms) * 0.08;
        }

        // Peak hold logic
        if (tPeak >= this.peakHold) {
            this.peakHold = tPeak;
            this.peakHoldTimer = 0;
        } else {
            this.peakHoldTimer++;
            if (this.peakHoldTimer > 45) { // ~0.75s hold then decay
                this.peakHold -= 0.008;
            }
        }
    }

    draw() {
        const ctx = this.ctx;
        ctx.clearRect(0, 0, this.w, this.h);

        // Background
        ctx.fillStyle = METER_COLORS.bg;
        ctx.fillRect(0, 0, this.w, this.h);

        if (this.orientation === 'vertical') {
            this.drawVertical();
        } else {
            this.drawHorizontal();
        }

        // Clip indicator
        if (this.clipped) {
            ctx.fillStyle = METER_COLORS.red;
            if (this.orientation === 'vertical') {
                ctx.fillRect(0, 0, this.w, 3);
            } else {
                ctx.fillRect(this.w - 3, 0, 3, this.h);
            }
        }
    }

    drawVertical() {
        const ctx = this.ctx;
        const h = this.h;
        const w = this.w;

        // RMS bar (thinner, behind peak)
        const rmsH = this.displayRms * h;
        const rmsY = h - rmsH;
        ctx.fillStyle = METER_COLORS.green;
        ctx.fillRect(0, rmsY, w, rmsH);

        // Peak bar gradient
        const peakH = this.displayPeak * h;
        const peakY = h - peakH;
        const grad = ctx.createLinearGradient(0, h, 0, 0);
        grad.addColorStop(0, METER_COLORS.green);
        grad.addColorStop(0.6, METER_COLORS.green);
        grad.addColorStop(0.8, METER_COLORS.yellow);
        grad.addColorStop(0.95, METER_COLORS.red);
        ctx.fillStyle = grad;
        ctx.fillRect(0, peakY, w, peakH);

        // Peak hold marker (thin white line)
        if (this.peakHold > 0.01) {
            const holdY = h - this.peakHold * h;
            ctx.fillStyle = '#fff';
            ctx.fillRect(0, holdY - 1, w, 2);
        }
    }

    drawHorizontal() {
        const ctx = this.ctx;
        const w = this.w;
        const h = this.h;

        // RMS bar
        const rmsW = this.displayRms * w;
        ctx.fillStyle = METER_COLORS.green;
        ctx.fillRect(0, 0, rmsW, h);

        // Peak bar gradient
        const peakW = this.displayPeak * w;
        const grad = ctx.createLinearGradient(0, 0, w, 0);
        grad.addColorStop(0, METER_COLORS.green);
        grad.addColorStop(0.6, METER_COLORS.green);
        grad.addColorStop(0.8, METER_COLORS.yellow);
        grad.addColorStop(0.95, METER_COLORS.red);
        ctx.fillStyle = grad;
        ctx.fillRect(0, 0, peakW, h);

        // Peak hold marker
        if (this.peakHold > 0.01) {
            const holdX = this.peakHold * w;
            ctx.fillStyle = '#fff';
            ctx.fillRect(holdX - 1, 0, 2, h);
        }
    }
}

// Registry of all meters
const meters = [];
let rafId = null;

export function registerMeter(canvas, orientation = 'vertical') {
    const m = new MeterBar(canvas, orientation);
    meters.push(m);
    startRAF();
    return m;
}

export function getMeter(index) {
    return meters[index];
}

function startRAF() {
    if (rafId !== null) return;
    function loop() {
        for (const m of meters) {
            m.update();
            m.draw();
        }
        rafId = requestAnimationFrame(loop);
    }
    rafId = requestAnimationFrame(loop);
}

export { dbToNorm };
