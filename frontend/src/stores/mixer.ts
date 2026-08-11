// stores/mixer.ts — Reactive state for the WASM mixer
import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

export const NUM_CHANNELS = 8;

export interface ChannelState {
  gain: number;       // linear 0-2 (fader position maps via dB)
  pan: number;        // -1 to 1
  muted: boolean;
  soloed: boolean;
  eqBypassed: boolean;
  eqBands: { gainDb: number }[];  // 4 main bands
}

export interface MeterData {
  peakL: number;
  peakR: number;
  rmsL: number;
  rmsR: number;
  clip: boolean;
}

export const [channels, setChannels] = createStore<ChannelState[]>(
  Array.from({ length: NUM_CHANNELS }, (_, i) => ({
    gain: 1.0,
    pan: 0,
    muted: false,
    soloed: false,
    eqBypassed: false,
    eqBands: [
      { gainDb: 0 },
      { gainDb: 0 },
      { gainDb: 0 },
      { gainDb: 0 },
    ],
  }))
);

export const [meterData, setMeterData] = createSignal<MeterData>({
  peakL: -Infinity, peakR: -Infinity,
  rmsL: -Infinity, rmsR: -Infinity,
  clip: false,
});

export const [wasmReady, setWasmReady] = createSignal(false);
export const [isRunning, setIsRunning] = createSignal(false);
export const [status, setStatus] = createSignal("Loading…");

// WASM message bus
let mixerNode: AudioWorkletNode | null = null;

export function setMixerNode(node: AudioWorkletNode) {
  mixerNode = node;
}

export function sendToWorklet(msg: any) {
  if (mixerNode) mixerNode.port.postMessage(msg);
}

// dB ↔ fader position conversion (logarithmic, -60 to +6 dB)
export function faderToGain(pos: number): number {
  const db = -60 + pos * 66;
  if (db <= -59) return 0;
  return Math.pow(10, db / 20);
}

export function gainToFader(gain: number): number {
  if (gain <= 0.001) return 0;
  const db = 20 * Math.log10(gain);
  return Math.max(0, Math.min(1, (db + 60) / 66));
}

export function formatGainDb(gain: number): string {
  if (gain <= 0.001) return "−∞";
  const db = 20 * Math.log10(gain);
  return db >= 0 ? `+${db.toFixed(1)}` : db.toFixed(1);
}
