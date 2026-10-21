import { onMount, onCleanup, createEffect } from "solid-js";

function dbToNorm(db: number): number {
  if (db <= -60) return 0;
  if (db >= 0) return 1;
  return (db + 60) / 60;
}

// Paint is skipped once every drawn quantity is within this distance of
// its target: the asymptotic decay never exactly settles, so without the
// cutoff the loop repaints identical frames forever (~0.3px at h=160).
const SETTLE_EPS = 0.002;

export function MeterCanvas(props: { width?: number; height?: number; peakDb?: number; rmsDb?: number }) {
  let canvas: HTMLCanvasElement | undefined;
  let rafId = 0;
  let displayPeak = 0;
  let displayRms = 0;
  let peakHold = 0;
  let peakHoldTimer = 0;
  let targetPeak = 0;
  let targetRms = 0;
  let painted = false;
  // Assigned in onMount (closes over ctx); wakes the loop after an idle stop.
  let wake = () => {};

  onMount(() => {
    if (!canvas) return;
    const ctx = canvas.getContext("2d")!;
    const dpr = window.devicePixelRatio || 1;
    const w = props.width || 10;
    const h = props.height || 130;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.scale(dpr, dpr);

    // Constant per instance — hoisted out of the paint loop.
    const grad = ctx.createLinearGradient(0, h, 0, 0);
    grad.addColorStop(0, "#22c55e");
    grad.addColorStop(0.6, "#22c55e");
    grad.addColorStop(0.8, "#eab308");
    grad.addColorStop(0.95, "#ef4444");

    function loop() {
      rafId = 0;
      if (!canvas) return;
      if (targetPeak > displayPeak) { displayPeak += (targetPeak - displayPeak) * 0.3; }
      else { displayPeak += (targetPeak - displayPeak) * 0.06; }
      if (targetRms > displayRms) { displayRms += (targetRms - displayRms) * 0.3; }
      else { displayRms += (targetRms - displayRms) * 0.06; }
      if (targetPeak >= peakHold) { peakHold = targetPeak; peakHoldTimer = 0; }
      else { peakHoldTimer++; if (peakHoldTimer > 45) peakHold -= 0.008; }

      // Idle cutoff: both displayed levels are at their targets and the
      // peak-hold line is either gone or resting on the current peak.
      // Leave the last frame on the canvas and stop scheduling; wake()
      // restarts the loop when the targets change again. A peak held
      // ABOVE the current level deliberately keeps the loop alive so the
      // hold line still falls after its timer.
      if (painted
        && Math.abs(targetPeak - displayPeak) < SETTLE_EPS
        && Math.abs(targetRms - displayRms) < SETTLE_EPS
        && (peakHold <= 0.01 || Math.abs(targetPeak - peakHold) < SETTLE_EPS)) {
        return;
      }

      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = "rgba(0,0,0,0.6)";
      ctx.fillRect(0, 0, w, h);

      const rmsH = displayRms * h;
      ctx.fillStyle = "#22c55e";
      ctx.fillRect(0, h - rmsH, w, rmsH);

      const peakH = displayPeak * h;
      ctx.fillStyle = grad;
      ctx.fillRect(0, h - peakH, w, peakH);

      if (peakHold > 0.01) {
        ctx.fillStyle = "#fff";
        ctx.fillRect(0, h - peakHold * h - 1, w, 2);
      }
      painted = true;
      rafId = requestAnimationFrame(loop);
    }

    wake = () => { if (!rafId) rafId = requestAnimationFrame(loop); };
    wake();
  });

  onCleanup(() => cancelAnimationFrame(rafId));

  createEffect(() => {
    const p = props.peakDb ?? -Infinity;
    const r = props.rmsDb ?? -Infinity;
    targetPeak = dbToNorm(p);
    targetRms = dbToNorm(r);
    wake();
  });

  return (
    <canvas
      ref={canvas}
      style={{ width: `${props.width || 10}px`, height: `${props.height || 130}px` }}
    />
  );
}
