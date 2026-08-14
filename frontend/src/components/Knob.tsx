import { onMount, onCleanup, createEffect } from "solid-js";

export interface KnobProps {
  label: string;           // short label shown above (e.g. "THR", "RATIO", "FREQ")
  value: number;           // current value
  min: number;             // minimum value
  max: number;             // maximum value
  defaultValue: number;    // value to reset to on double-click
  onChange: (value: number) => void;  // called with new value during drag
  format?: (value: number) => string; // format the value for display (default: rounded number)
  unit?: string;           // unit string appended to formatted value (e.g. "dB", "ms", "Hz")
  size?: number;           // knob diameter in pixels (default: 36)
  color?: string;          // indicator arc color (default: "#4a8fdd")
  log?: boolean;           // logarithmic scale (for frequency knobs). Default: false (linear)
}

// Arc geometry (canvas coords: 0 rad = 3 o'clock, angles increase clockwise):
//   start = 0.75π  -> 7:30 position (bottom-left)
//   end   = 2.25π  -> 4:30 position (bottom-right)
//   total sweep    = 1.5π (270°), leaving a bottom gap
const ARC_START = 0.75 * Math.PI;
const ARC_SWEEP = 1.5 * Math.PI;

// Normalize a value to 0..1, honoring linear/log scale.
// Log mode uses true logarithm for positive-only ranges; for ranges that
// include negative values (e.g. dB thresholds min=-80) a power-curve
// approximation gives more resolution near the bottom of the travel.
function normalizeValue(value: number, min: number, max: number, log: boolean): number {
  if (max <= min) return 0;
  const t = (value - min) / (max - min);
  if (!log) return t;
  if (min > 0 && max > 0) {
    const v = value <= 0 ? min : value;
    return Math.log(v / min) / Math.log(max / min);
  }
  return Math.pow(Math.max(0, t), 0.4);
}

// Inverse of normalizeValue: map a 0..1 position back to the value range.
function denormalizeValue(t: number, min: number, max: number, log: boolean): number {
  if (max <= min) return min;
  t = Math.max(0, Math.min(1, t));
  if (!log) return min + t * (max - min);
  if (min > 0 && max > 0) {
    return min * Math.pow(max / min, t);
  }
  return min + Math.pow(t, 2.5) * (max - min);
}

export function Knob(props: KnobProps) {
  let canvas: HTMLCanvasElement | undefined;

  const size = () => props.size ?? 36;
  const color = () => props.color ?? "#4a8fdd";
  const isLog = () => props.log ?? false;
  const unit = () => props.unit ?? "";

  const fmt = (v: number) => props.format ? props.format(v) : String(Math.round(v));
  const displayValue = () => `${fmt(props.value)}${unit()}`;
  const tooltip = () => `${props.label}: ${fmt(props.value)}${unit()}`;

  // Normalize current value to 0..1, honoring linear/log scale.
  function normalized(): number {
    return normalizeValue(props.value, props.min, props.max, isLog());
  }

  function draw() {
    if (!canvas) return;
    const ctx = canvas.getContext("2d")!;
    const s = size();
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== s * dpr) {
      canvas.width = s * dpr;
      canvas.height = s * dpr;
      ctx.scale(dpr, dpr);
    }

    ctx.clearRect(0, 0, s, s);
    ctx.lineCap = "round";

    const cx = s / 2;
    const cy = s / 2;
    const rOuter = s / 2;
    const rBg = Math.max(1, rOuter - 1);
    const rArc = Math.max(1, rOuter - 2.5);
    const rInner = Math.max(1, rOuter - 4.5);
    const arcLw = Math.max(1.5, s * 0.08);
    const norm = Math.max(0, Math.min(1, normalized()));

    // 1. Background circle (dark fill + border)
    ctx.beginPath();
    ctx.arc(cx, cy, rBg, 0, Math.PI * 2);
    ctx.fillStyle = "#1a1a25";
    ctx.fill();
    ctx.lineWidth = 1;
    ctx.strokeStyle = "#2a2a35";
    ctx.stroke();

    // 2. Arc indicator: dim track for full sweep, then filled portion in `color`.
    ctx.beginPath();
    ctx.arc(cx, cy, rArc, ARC_START, ARC_START + ARC_SWEEP, false);
    ctx.lineWidth = arcLw;
    ctx.strokeStyle = "#2a2a35";
    ctx.stroke();

    if (norm > 0.001) {
      ctx.beginPath();
      ctx.arc(cx, cy, rArc, ARC_START, ARC_START + norm * ARC_SWEEP, false);
      ctx.lineWidth = arcLw;
      ctx.strokeStyle = color();
      ctx.stroke();
    }

    // 3. Center fill (slightly lighter circle inside)
    ctx.beginPath();
    ctx.arc(cx, cy, rInner, 0, Math.PI * 2);
    ctx.fillStyle = "#16161e";
    ctx.fill();

    // 4. Indicator line from center to the arc at the current value's angle.
    const valAngle = ARC_START + norm * ARC_SWEEP;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + rArc * Math.cos(valAngle), cy + rArc * Math.sin(valAngle));
    ctx.lineWidth = 2;
    ctx.strokeStyle = "#ccc";
    ctx.stroke();
  }

  // ── Drag interaction (vertical delta, DAW-style) ───────────────────────
  let startY = 0;
  let startValue = 0;
  let dragging = false;

  function onDocMouseMove(e: MouseEvent) {
    if (!dragging) return;
    const delta = startY - e.clientY;            // positive when dragging up
    const sensitivity = e.shiftKey ? 500 : 100;  // px for full range (Shift = 5× finer)
    const nd = delta / sensitivity;
    const log = isLog();
    const startNorm = normalizeValue(startValue, props.min, props.max, log);
    const newValue = denormalizeValue(startNorm + nd, props.min, props.max, log);
    props.onChange(newValue);
  }

  function onDocMouseUp() {
    dragging = false;
    document.removeEventListener("mousemove", onDocMouseMove);
    document.removeEventListener("mouseup", onDocMouseUp);
  }

  function onMouseDown(e: MouseEvent) {
    e.preventDefault();  // avoid text selection
    startY = e.clientY;
    startValue = props.value;
    dragging = true;
    document.addEventListener("mousemove", onDocMouseMove);
    document.addEventListener("mouseup", onDocMouseUp);
  }

  function onDoubleClick(e: MouseEvent) {
    e.preventDefault();
    props.onChange(props.defaultValue);
  }

  onMount(() => draw());

  // Redraw whenever any prop read inside draw() changes.
  createEffect(() => draw());

  onCleanup(() => {
    document.removeEventListener("mousemove", onDocMouseMove);
    document.removeEventListener("mouseup", onDocMouseUp);
  });

  return (
    <div
      class="knob-container"
      title={tooltip()}
      onMouseDown={onMouseDown}
      onDblClick={onDoubleClick}
    >
      <span class="knob-label">{props.label}</span>
      <canvas
        ref={canvas}
        class="knob-canvas"
        style={{ width: `${size()}px`, height: `${size()}px` }}
      />
      <span class="knob-value">{displayValue()}</span>
    </div>
  );
}
