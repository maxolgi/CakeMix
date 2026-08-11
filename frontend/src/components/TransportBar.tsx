import { Show } from "solid-js";
import type { MeterData } from "../stores/mixer";

function dbToNorm(db: number): number {
  if (db <= -60) return 0;
  if (db >= 0) return 1;
  return (db + 60) / 60;
}

function fmtDb(db: number): string {
  return db <= -60 ? "−∞" : db.toFixed(1);
}

export function TransportBar(props: {
  running: boolean;
  wasmReady: boolean;
  meter: MeterData;
  onStart: () => void;
  onStop: () => void;
}) {
  return (
    <div class="transport-bar">
      <button
        class="btn btn-start"
        disabled={!props.wasmReady || props.running}
        onClick={props.onStart}
      >Start</button>
      <button
        class="btn btn-stop"
        disabled={!props.running}
        onClick={props.onStop}
      >Stop</button>

      <div class="master-meters">
        <div class="h-meter-row">
          <span class="h-meter-label">L</span>
          <div class="h-meter-track">
            <div
              class="h-meter-bar"
              style={{ width: `${dbToNorm(props.meter.peakL) * 100}%` }}
            />
          </div>
        </div>
        <div class="h-meter-row">
          <span class="h-meter-label">R</span>
          <div class="h-meter-track">
            <div
              class="h-meter-bar"
              style={{ width: `${dbToNorm(props.meter.peakR) * 100}%` }}
            />
          </div>
        </div>
        <span class="meter-readout">
          {fmtDb(props.meter.rmsL)}/{fmtDb(props.meter.peakL)} |
          {" "}{fmtDb(props.meter.rmsR)}/{fmtDb(props.meter.peakR)} dB
        </span>
        <span class={`clip ${props.meter.clip ? "active" : ""}`}>CLIP</span>
      </div>
    </div>
  );
}
