import { onMount, onCleanup, createEffect } from "solid-js";
import { meterData } from "../stores/mixer";

// dB → normalized (0 = -60 dB, 1 = 0 dB)
function dbToNorm(db: number): number {
  if (db <= -60) return 0;
  if (db >= 0) return 1;
  return (db + 60) / 60;
}

export function MeterCanvas(props: { width?: number; height?: number }) {
  let canvas: HTMLCanvasElement | undefined;
  let rafId = 0;

  // Smoothed display values
  let displayPeak = 0;
  let displayRms = 0;
  let peakHold = 0;
  let peakHoldTimer = 0;
  let targetPeak = 0;
  let targetRms = 0;

  onMount(() => {
    if (!canvas) return;
    const ctx = canvas.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    const w = props.width || 10;
    const h = props.height || 130;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.scale(dpr, dpr);

    function loop() {
      if (!canvas) return;
      // Interpolate toward targets
      if (targetPeak > displayPeak) {
        displayPeak += (targetPeak - displayPeak) * 0.3;
      } else {
        displayPeak += (targetPeak - displayPeak) * 0.06;
      }
      if (targetRms > displayRms) {
        displayRms += (targetRms - displayRms) * 0.3;
      } else {
        displayRms += (targetRms - displayRms) * 0.06;
      }

      // Peak hold
      if (targetPeak >= peakHold) {
        peakHold = targetPeak;
        peakHoldTimer = 0;
      } else {
        peakHoldTimer++;
        if (peakHoldTimer > 45) peakHold -= 0.008;
      }

      // Draw
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = "rgba(0,0,0,0.6)";
      ctx.fillRect(0, 0, w, h);

      // RMS bar (green)
      const rmsH = displayRms * h;
      ctx.fillStyle = "#22c55e";
      ctx.fillRect(0, h - rmsH, w, rmsH);

      // Peak bar (gradient)
      const peakH = displayPeak * h;
      const grad = ctx.createLinearGradient(0, h, 0, 0);
      grad.addColorStop(0, "#22c55e");
      grad.addColorStop(0.6, "#22c55e");
      grad.addColorStop(0.8, "#eab308");
      grad.addColorStop(0.95, "#ef4444");
      ctx.fillStyle = grad;
      ctx.fillRect(0, h - peakH, w, peakH);

      // Peak hold marker
      if (peakHold > 0.01) {
        ctx.fillStyle = "#fff";
        ctx.fillRect(0, h - peakHold * h - 1, w, 2);
      }

      rafId = requestAnimationFrame(loop);
    }
    rafId = requestAnimationFrame(loop);
  });

  onCleanup(() => cancelAnimationFrame(rafId));

  // Update targets from master meter data
  // Since we only have master meters, approximate per-channel
  createEffect(() => {
    const m = meterData();
    if (m.peakL > -60) {
      targetPeak = dbToNorm(m.peakL - 6);
      targetRms = dbToNorm(m.rmsL - 9);
    } else {
      targetPeak = 0;
      targetRms = 0;
    }
  });

  return (
    <canvas
      ref={canvas}
      style={{ width: `${props.width || 10}px`, height: `${props.height || 130}px` }}
    />
  );
}
