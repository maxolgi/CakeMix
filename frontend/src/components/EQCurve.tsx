import { onMount, onCleanup, createEffect, on } from "solid-js";
import { channels } from "../stores/mixer";

// Biquad filter coefficients for each EQ band type
function biquadResponse(type: string, f0: number, q: number, dbGain: number, freq: number, sampleRate: number): number {
  if (type === "none" || dbGain === 0) return 0;
  const fs = sampleRate;
  const A = Math.pow(10, dbGain / 40);
  const w0 = (2 * Math.PI * f0) / fs;
  const alpha = Math.sin(w0) / (2 * q);

  let b0 = 0, b1 = 0, b2 = 0, a0 = 0, a1 = 0, a2 = 0;

  switch (type) {
    case "peak": case "peaking":
      b0 = 1 + alpha * A; b1 = -2 * Math.cos(w0); b2 = 1 - alpha * A;
      a0 = 1 + alpha / A; a1 = -2 * Math.cos(w0); a2 = 1 - alpha / A;
      break;
    case "low_shelf": case "lowshelf":
      b0 = A * (A + 1 - (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha);
      b1 = 2 * A * (A - 1 - (A + 1) * Math.cos(w0));
      b2 = A * (A + 1 - (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha);
      a0 = A + 1 + (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha;
      a1 = -2 * (A - 1 + (A + 1) * Math.cos(w0));
      a2 = A + 1 + (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha;
      break;
    case "high_shelf": case "highshelf":
      b0 = A * (A + 1 + (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha);
      b1 = -2 * A * (A - 1 + (A + 1) * Math.cos(w0));
      b2 = A * (A + 1 + (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha);
      a0 = A + 1 - (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha;
      a1 = 2 * (A - 1 - (A + 1) * Math.cos(w0));
      a2 = A + 1 - (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha;
      break;
    case "high_pass": case "highpass":
      b0 = (1 + Math.cos(w0)) / 2; b1 = -(1 + Math.cos(w0)); b2 = (1 + Math.cos(w0)) / 2;
      a0 = 1 + alpha; a1 = -2 * Math.cos(w0); a2 = 1 - alpha;
      break;
    default:
      return 0;
  }

  // Normalize and compute magnitude response
  const b0n = b0 / a0, b1n = b1 / a0, b2n = b2 / a0;
  const a1n = a1 / a0, a2n = a2 / a0;
  const w = (2 * Math.PI * freq) / fs;
  const cosw = Math.cos(w), sinw = Math.sin(w);
  const cos2w = Math.cos(2 * w), sin2w = Math.sin(2 * w);

  const numeratorReal = b0n + b1n * cosw + b2n * cos2w;
  const numeratorImag = -b1n * sinw - b2n * sin2w;
  const denomReal = 1 + a1n * cosw + a2n * cos2w;
  const denomImag = -a1n * sinw - a2n * sin2w;

  const mag = Math.sqrt(
    (numeratorReal * numeratorReal + numeratorImag * numeratorImag) /
    (denomReal * denomReal + denomImag * denomImag)
  );
  return 20 * Math.log10(mag);
}

const SAMPLE_RATE = 48000;
const BAND_INFO = [
  { type: "low_shelf", freq: 120, q: 0.707 },
  { type: "peak", freq: 400, q: 1.0 },
  { type: "peak", freq: 1500, q: 1.0 },
  { type: "peak", freq: 5000, q: 1.0 },
];

export function EQCurve(props: { channelIndex: number }) {
  let canvas: HTMLCanvasElement | undefined;

  function draw() {
    if (!canvas) return;
    const ctx = canvas.getContext("2d")!;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== w * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      ctx.scale(dpr, dpr);
    }

    ctx.clearRect(0, 0, w, h);

    // Grid lines
    ctx.strokeStyle = "rgba(255,255,255,0.05)";
    ctx.lineWidth = 1;
    // 0 dB line (center)
    ctx.strokeStyle = "rgba(255,255,255,0.15)";
    ctx.beginPath();
    ctx.moveTo(0, h / 2);
    ctx.lineTo(w, h / 2);
    ctx.stroke();

    // Frequency grid (log scale)
    ctx.strokeStyle = "rgba(255,255,255,0.03)";
    [50, 100, 200, 500, 1000, 2000, 5000, 10000].forEach(f => {
      const x = freqToX(f, w);
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    });

    // Build frequency array and compute response
    const ch = channels[props.channelIndex];
    if (ch.eqBypassed) {
      ctx.strokeStyle = "#555";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(0, h / 2);
      ctx.lineTo(w, h / 2);
      ctx.stroke();
      return;
    }

    const numPoints = 200;
    const minLog = Math.log10(20);
    const maxLog = Math.log10(SAMPLE_RATE / 2);
    const dbRange = 18; // ±18 dB display range

    ctx.strokeStyle = "#4a8fdd";
    ctx.lineWidth = 1.5;
    ctx.beginPath();

    for (let i = 0; i <= numPoints; i++) {
      const logVal = minLog + (i / numPoints) * (maxLog - minLog);
      const freq = Math.pow(10, logVal);
      const x = (i / numPoints) * w;

      // Sum all band responses
      let totalDb = 0;
      for (let b = 0; b < BAND_INFO.length; b++) {
        const band = BAND_INFO[b];
        const gainDb = ch.eqBands[b]?.gainDb || 0;
        if (Math.abs(gainDb) > 0.01) {
          totalDb += biquadResponse(band.type, band.freq, band.q, gainDb, freq, SAMPLE_RATE);
        }
      }

      // Map dB to y: 0 dB = center, +18 dB = top, -18 dB = bottom
      const y = h / 2 - (totalDb / dbRange) * (h / 2);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Fill area under curve
    ctx.lineTo(w, h / 2);
    ctx.lineTo(0, h / 2);
    ctx.closePath();
    ctx.fillStyle = "rgba(74, 143, 221, 0.1)";
    ctx.fill();
  }

  function freqToX(freq: number, w: number): number {
    const minLog = Math.log10(20);
    const maxLog = Math.log10(SAMPLE_RATE / 2);
    return ((Math.log10(freq) - minLog) / (maxLog - minLog)) * w;
  }

  // Redraw when EQ params change
  createEffect(on(
    () => channels[props.channelIndex]?.eqBands.map(b => b.gainDb).join(","),
    () => requestAnimationFrame(draw)
  ));

  createEffect(on(
    () => channels[props.channelIndex]?.eqBypassed,
    () => requestAnimationFrame(draw)
  ));

  onMount(() => draw());
  onCleanup(() => {});

  return (
    <canvas
      ref={canvas}
      class="eq-curve"
      style={{ width: "100%", height: "50px" }}
    />
  );
}
