import type { MeterData } from "../stores/mixer";

function dbToNorm(db: number): number {
  if (db <= -60) return 0;
  if (db >= 0) return 1;
  return (db + 60) / 60;
}

export function MasterStrip(props: { meter: MeterData }) {
  return (
    <div class="channel-strip master-strip">
      <div class="ch-header">
        <span class="ch-num">MST</span>
      </div>
      <div class="master-label">MASTER</div>
      <div class="ch-meter-col">
        <canvas
          class="master-meter-canvas-l"
          style={{ width: "10px", height: "130px" }}
        />
        <canvas
          class="master-meter-canvas-r"
          style={{ width: "10px", height: "130px" }}
        />
      </div>
      <div class="fader-section">
        <span class="fader-val">0.0</span>
        <input
          type="range"
          class="fader"
          min="0" max="1" step="0.001" value="0.85"
          style={{ "writing-mode": "vertical-lr", direction: "rtl" }}
        />
      </div>
    </div>
  );
}
