import { onMount, onCleanup, createEffect, on } from "solid-js";
import { channels, busChannels } from "../stores/mixer";

function biquadResponse(type: string, f0: number, q: number, dbGain: number, freq: number, sampleRate: number): number {
  if (type === "none" || dbGain === 0) return 0;
  const A = Math.pow(10, dbGain / 40);
  const w0 = (2 * Math.PI * f0) / sampleRate;
  const alpha = Math.sin(w0) / (2 * q);
  let b0 = 0, b1 = 0, b2 = 0, a0 = 0, a1 = 0, a2 = 0;

  switch (type) {
    case "peak": case "peaking":
      b0 = 1 + alpha * A; b1 = -2 * Math.cos(w0); b2 = 1 - alpha * A;
      a0 = 1 + alpha / A; a1 = -2 * Math.cos(w0); a2 = 1 - alpha / A; break;
    case "low_shelf": case "lowshelf":
      b0 = A * (A + 1 - (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha);
      b1 = 2 * A * (A - 1 - (A + 1) * Math.cos(w0));
      b2 = A * (A + 1 - (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha);
      a0 = A + 1 + (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha;
      a1 = -2 * (A - 1 + (A + 1) * Math.cos(w0));
      a2 = A + 1 + (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha; break;
    case "high_shelf": case "highshelf":
      b0 = A * (A + 1 + (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha);
      b1 = -2 * A * (A - 1 + (A + 1) * Math.cos(w0));
      b2 = A * (A + 1 + (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha);
      a0 = A + 1 - (A - 1) * Math.cos(w0) + 2 * Math.sqrt(A) * alpha;
      a1 = 2 * (A - 1 - (A + 1) * Math.cos(w0));
      a2 = A + 1 - (A - 1) * Math.cos(w0) - 2 * Math.sqrt(A) * alpha; break;
    case "high_pass": case "highpass":
      b0 = (1 + Math.cos(w0)) / 2; b1 = -(1 + Math.cos(w0)); b2 = (1 + Math.cos(w0)) / 2;
      a0 = 1 + alpha; a1 = -2 * Math.cos(w0); a2 = 1 - alpha;
      if (dbGain === 0) return 0; break;
    default: return 0;
  }

  const b0n = b0 / a0, b1n = b1 / a0, b2n = b2 / a0;
  const a1n = a1 / a0, a2n = a2 / a0;
  const w = (2 * Math.PI * freq) / sampleRate;
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
  { type: "high_pass", freq: 80, q: 0.707 },
  { type: "low_shelf", freq: 120, q: 0.707 },
  { type: "peak", freq: 400, q: 1.0 },
  { type: "peak", freq: 1500, q: 1.0 },
  { type: "peak", freq: 5000, q: 1.0 },
  { type: "high_shelf", freq: 10000, q: 0.707 },
];

export function EQCurve(props: { channelIndex: number; bus?: boolean }) {
  let canvas: HTMLCanvasElement | undefined;

  // When `bus` is set, channelIndex selects a bus; otherwise a channel.
  const eqState = () => props.bus ? busChannels[props.channelIndex] : channels[props.channelIndex];

  function draw() {
    if (!canvas) return;
    const ctx = canvas.getContext("2d")!;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== w * dpr) { canvas.width = w * dpr; canvas.height = h * dpr; ctx.scale(dpr, dpr); }
    ctx.clearRect(0, 0, w, h);

    ctx.strokeStyle = "rgba(255,255,255,0.15)";
    ctx.beginPath(); ctx.moveTo(0, h / 2); ctx.lineTo(w, h / 2); ctx.stroke();

    ctx.strokeStyle = "rgba(255,255,255,0.03)";
    [50, 100, 200, 500, 1000, 2000, 5000, 10000].forEach(f => {
      const x = freqToX(f, w);
      ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, h); ctx.stroke();
    });

    const ch = eqState();
    if (!ch) return;
    if (ch.eqBypassed) {
      ctx.strokeStyle = "#555"; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(0, h / 2); ctx.lineTo(w, h / 2); ctx.stroke(); return;
    }

    const numPoints = 200;
    const minLog = Math.log10(20);
    const maxLog = Math.log10(SAMPLE_RATE / 2);
    const dbRange = 18;

    ctx.strokeStyle = "#4a8fdd"; ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i <= numPoints; i++) {
      const logVal = minLog + (i / numPoints) * (maxLog - minLog);
      const freq = Math.pow(10, logVal);
      const x = (i / numPoints) * w;
      let totalDb = 0;
      for (let b = 0; b < BAND_INFO.length; b++) {
        const band = BAND_INFO[b];
        const gainDb = ch.eqBands[b]?.gainDb || 0;
        const bandFreq = ch.eqBands[b]?.freqHz || band.freq;
        const bandQ = ch.eqBands[b]?.q || band.q;
        if (Math.abs(gainDb) > 0.01 || band.type === "high_pass") {
          totalDb += biquadResponse(band.type, bandFreq, bandQ, gainDb, freq, SAMPLE_RATE);
        }
      }
      const y = h / 2 - (totalDb / dbRange) * (h / 2);
      if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    }
    ctx.stroke();
    ctx.lineTo(w, h / 2); ctx.lineTo(0, h / 2); ctx.closePath();
    ctx.fillStyle = "rgba(74, 143, 221, 0.1)"; ctx.fill();
  }

  function freqToX(freq: number, w: number): number {
    const minLog = Math.log10(20);
    const maxLog = Math.log10(SAMPLE_RATE / 2);
    return ((Math.log10(freq) - minLog) / (maxLog - minLog)) * w;
  }

  createEffect(on(() => eqState()?.eqBands.flatMap(b => [b.gainDb, b.freqHz, b.q]).join(","),
    () => requestAnimationFrame(draw)));
  createEffect(on(() => eqState()?.eqBypassed, () => requestAnimationFrame(draw)));
  onMount(() => draw());
  onCleanup(() => {});

  return <canvas ref={canvas} class="eq-curve" style={{ width: "100%", height: "50px" }} />;
}
