import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

export const NUM_CHANNELS = 128;

export interface EqBand { gainDb: number; freqHz: number; q: number; }
export interface AuxSend { levelDb: number; preFader: boolean; busId: number | null; }
export interface ChannelMeter { ch: number; peak: number; rms: number; }

export interface BusState {
  name: string;
  sources: (number | null)[];  // 16 slots
  gain: number;
  muted: boolean;
  eqBypassed: boolean;
  eqBands: EqBand[];
  gateEnabled: boolean; gateThresholdDb: number; gateHysteresisDb: number;
  gateAttackMs: number; gateReleaseMs: number; gateHoldMs: number;
  compEnabled: boolean; compThresholdDb: number; compRatio: number; compKneeDb: number;
  compAttackMs: number; compReleaseMs: number; compMakeupDb: number;
  expanderEnabled: boolean; expanderThresholdDb: number; expanderRatio: number;
  expanderAttackMs: number; expanderReleaseMs: number;
  peakDb: number; rmsDb: number;
}

export interface BusMeter { bus: number; peak: number; rms: number; }

export interface ChannelState {
  name: string;
  gain: number; inputGainDb: number; phaseInverted: boolean;
  pan: number; panLaw: number;
  muted: boolean; soloed: boolean;
  eqBypassed: boolean; eqBands: EqBand[];
  gateEnabled: boolean; gateThresholdDb: number; gateHysteresisDb: number;
  gateAttackMs: number; gateReleaseMs: number; gateHoldMs: number;
  compEnabled: boolean; compThresholdDb: number; compRatio: number; compKneeDb: number;
  compAttackMs: number; compReleaseMs: number; compMakeupDb: number;
  expanderEnabled: boolean; expanderThresholdDb: number; expanderRatio: number;
  expanderAttackMs: number; expanderReleaseMs: number;
  sends: AuxSend[];
  outputBus: number | "master";
  peakDb: number; rmsDb: number;
}

export interface MeterData {
  peakL: number; peakR: number; rmsL: number; rmsR: number;
  clip: boolean; limiterGr: number; channels: ChannelMeter[];
  buses: BusMeter[];
}

const EQ_BAND_DEFAULTS: EqBand[] = [
  { gainDb: 0, freqHz: 80, q: 0.707 },
  { gainDb: 0, freqHz: 120, q: 0.707 },
  { gainDb: 0, freqHz: 400, q: 1.0 },
  { gainDb: 0, freqHz: 1500, q: 1.0 },
  { gainDb: 0, freqHz: 5000, q: 1.0 },
  { gainDb: 0, freqHz: 10000, q: 0.707 },
];

function defaultChannel(index: number): ChannelState {
  return {
    name: `ch${index + 1}`,
    gain: 1.0, inputGainDb: 0, phaseInverted: false,
    pan: 0, panLaw: 0,
    muted: false, soloed: false,
    eqBypassed: false, eqBands: EQ_BAND_DEFAULTS.map(b => ({ ...b })),
    gateEnabled: false, gateThresholdDb: -50, gateHysteresisDb: 6, gateAttackMs: 2, gateReleaseMs: 100, gateHoldMs: 10,
    compEnabled: false, compThresholdDb: -12, compRatio: 3, compKneeDb: 3, compAttackMs: 5, compReleaseMs: 100, compMakeupDb: 3,
    expanderEnabled: false, expanderThresholdDb: -40, expanderRatio: 2, expanderAttackMs: 5, expanderReleaseMs: 100,
    sends: Array.from({ length: 4 }, () => ({ levelDb: -Infinity, preFader: false, busId: null })),
    outputBus: "master",
    peakDb: -Infinity, rmsDb: -Infinity,
  };
}

export const [channels, setChannels] = createStore<ChannelState[]>(
  Array.from({ length: NUM_CHANNELS }, (_, i) => defaultChannel(i))
);

function defaultBus(index: number): BusState {
  return {
    name: `Bus ${index + 1}`,
    sources: Array.from({ length: 16 }, () => null),
    gain: 1.0, muted: false,
    eqBypassed: false, eqBands: EQ_BAND_DEFAULTS.map(b => ({ ...b })),
    gateEnabled: false, gateThresholdDb: -50, gateHysteresisDb: 6, gateAttackMs: 2, gateReleaseMs: 100, gateHoldMs: 10,
    compEnabled: false, compThresholdDb: -12, compRatio: 3, compKneeDb: 3, compAttackMs: 5, compReleaseMs: 100, compMakeupDb: 3,
    expanderEnabled: false, expanderThresholdDb: -40, expanderRatio: 2, expanderAttackMs: 5, expanderReleaseMs: 100,
    peakDb: -Infinity, rmsDb: -Infinity,
  };
}

export const [busChannels, setBusChannels] = createStore<BusState[]>(
  Array.from({ length: 8 }, (_, i) => defaultBus(i))
);
export const [selectedBus, setSelectedBus] = createSignal<number | null>(null);

export const [meterData, setMeterData] = createSignal<MeterData>({
  peakL: -Infinity, peakR: -Infinity, rmsL: -Infinity, rmsR: -Infinity,
  clip: false, limiterGr: 0, channels: [], buses: [],
});
export const [wasmReady, setWasmReady] = createSignal(false);
export const [isRunning, setIsRunning] = createSignal(false);
export const [status, setStatus] = createSignal("Loading…");
export const [masterGain, setMasterGainState] = createSignal(1.0);
export const [limiterEnabled, setLimiterEnabledState] = createSignal(true);
export const [limiterCeiling, setLimiterCeilingState] = createSignal(-0.3);
export const [limiterRelease, setLimiterReleaseState] = createSignal(50);

let mixerNode: AudioWorkletNode | null = null;
export function setMixerNode(node: AudioWorkletNode) { mixerNode = node; }
export function sendToWorklet(msg: any) { if (mixerNode) mixerNode.port.postMessage(msg); }

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

export function updateMeterData(msg: MeterData) {
  setMeterData(msg);
  if (msg.channels) for (const cm of msg.channels) {
    setChannels(cm.ch, "peakDb", cm.peak);
    setChannels(cm.ch, "rmsDb", cm.rms);
  }
  if (msg.buses) for (const bm of msg.buses) {
    setBusChannels(bm.bus, "peakDb", bm.peak);
    setBusChannels(bm.bus, "rmsDb", bm.rms);
  }
}

// Store helpers — each updates store + sends worklet message
export function setInputGain(ch: number, gainDb: number) { setChannels(ch, "inputGainDb", gainDb); sendToWorklet({ type: "set-input-gain", ch, gainDb }); }
export function setPhase(ch: number, inverted: boolean) { setChannels(ch, "phaseInverted", inverted); sendToWorklet({ type: "set-phase", ch, inverted }); }
export function setPanLaw(ch: number, law: number) { setChannels(ch, "panLaw", law); sendToWorklet({ type: "set-pan-law", ch, law }); }
export function setChannelName(ch: number, name: string) { setChannels(ch, "name", name); sendToWorklet({ type: "set-name", ch, name }); }
export function setEqGain(ch: number, band: number, gainDb: number) { setChannels(ch, "eqBands", band, "gainDb", gainDb); sendToWorklet({ type: "set-eq-gain", ch, band, gainDb }); }
export function setEqFreq(ch: number, band: number, freqHz: number) { setChannels(ch, "eqBands", band, "freqHz", freqHz); sendToWorklet({ type: "set-eq-freq", ch, band, freqHz }); }
export function setEqQ(ch: number, band: number, q: number) { setChannels(ch, "eqBands", band, "q", q); sendToWorklet({ type: "set-eq-q", ch, band, q }); }
export function setEqBypass(ch: number, bypassed: boolean) { setChannels(ch, "eqBypassed", bypassed); sendToWorklet({ type: "set-eq-bypass", ch, bypassed }); }
export function setCompEnabled(ch: number, enabled: boolean) { setChannels(ch, "compEnabled", enabled); sendToWorklet({ type: enabled ? "enable-compressor" : "disable-compressor", ch }); }
export function setCompThreshold(ch: number, v: number) { setChannels(ch, "compThresholdDb", v); sendToWorklet({ type: "set-comp-param", ch, param: 0, value: v }); }
export function setCompRatio(ch: number, v: number) { setChannels(ch, "compRatio", v); sendToWorklet({ type: "set-comp-param", ch, param: 1, value: v }); }
export function setCompAttack(ch: number, v: number) { setChannels(ch, "compAttackMs", v); sendToWorklet({ type: "set-comp-param", ch, param: 2, value: v }); }
export function setCompRelease(ch: number, v: number) { setChannels(ch, "compReleaseMs", v); sendToWorklet({ type: "set-comp-param", ch, param: 3, value: v }); }
export function setCompMakeup(ch: number, v: number) { setChannels(ch, "compMakeupDb", v); sendToWorklet({ type: "set-comp-param", ch, param: 4, value: v }); }
export function setCompKnee(ch: number, v: number) { setChannels(ch, "compKneeDb", v); sendToWorklet({ type: "set-comp-param", ch, param: 5, value: v }); }
export function setGateEnabled(ch: number, enabled: boolean) { setChannels(ch, "gateEnabled", enabled); sendToWorklet({ type: enabled ? "enable-gate" : "disable-gate", ch }); }
export function setGateThreshold(ch: number, v: number) { setChannels(ch, "gateThresholdDb", v); sendToWorklet({ type: "set-gate-param", ch, param: 0, value: v }); }
export function setGateHysteresis(ch: number, v: number) { setChannels(ch, "gateHysteresisDb", v); sendToWorklet({ type: "set-gate-param", ch, param: 1, value: v }); }
export function setGateAttack(ch: number, v: number) { setChannels(ch, "gateAttackMs", v); sendToWorklet({ type: "set-gate-param", ch, param: 2, value: v }); }
export function setGateRelease(ch: number, v: number) { setChannels(ch, "gateReleaseMs", v); sendToWorklet({ type: "set-gate-param", ch, param: 3, value: v }); }
export function setGateHold(ch: number, v: number) { setChannels(ch, "gateHoldMs", v); sendToWorklet({ type: "set-gate-param", ch, param: 4, value: v }); }
export function setExpanderEnabled(ch: number, enabled: boolean) { setChannels(ch, "expanderEnabled", enabled); sendToWorklet({ type: enabled ? "enable-expander" : "disable-expander", ch }); }
export function setExpanderThreshold(ch: number, v: number) { setChannels(ch, "expanderThresholdDb", v); sendToWorklet({ type: "set-exp-param", ch, param: 0, value: v }); }
export function setExpanderRatio(ch: number, v: number) { setChannels(ch, "expanderRatio", v); sendToWorklet({ type: "set-exp-param", ch, param: 1, value: v }); }
export function setExpanderAttack(ch: number, v: number) { setChannels(ch, "expanderAttackMs", v); sendToWorklet({ type: "set-exp-param", ch, param: 2, value: v }); }
export function setExpanderRelease(ch: number, v: number) { setChannels(ch, "expanderReleaseMs", v); sendToWorklet({ type: "set-exp-param", ch, param: 3, value: v }); }
export function setAuxSend(ch: number, sendIdx: number, levelDb: number, preFader: boolean, busId: number | null) {
  setChannels(ch, "sends", sendIdx, { levelDb, preFader, busId });
  if (busId !== null) sendToWorklet({ type: "set-aux-send", ch, sendIdx, busId, level: Math.pow(10, levelDb / 20), preFader });
}
export function routeToBus(ch: number, busId: number) { setChannels(ch, "outputBus", busId); sendToWorklet({ type: "route-to-bus", ch, busId }); }
export function routeToMaster(ch: number) { setChannels(ch, "outputBus", "master"); sendToWorklet({ type: "route-to-master", ch }); }

export function setMasterGain(gain: number) { setMasterGainState(gain); sendToWorklet({ type: "set-master-gain", gain }); }
export function setLimiterEnabled(enabled: boolean) { setLimiterEnabledState(enabled); sendToWorklet({ type: "set-limiter", enabled }); }
export function setLimiterCeiling(ceilingDb: number) { setLimiterCeilingState(ceilingDb); sendToWorklet({ type: "set-limiter-ceiling", ceilingDb }); }
export function setLimiterRelease(releaseMs: number) { setLimiterReleaseState(releaseMs); sendToWorklet({ type: "set-limiter-release", releaseMs }); }

// ── Bus helpers ──────────────────────────────────────────────────────────────

// Source routing
export function setBusSource(bus: number, slot: number, ch: number) {
  setBusChannels(bus, "sources", slot, ch);
  sendToWorklet({ type: "set-bus-source", bus, slot, ch });
}
export function clearBusSource(bus: number, slot: number) {
  setBusChannels(bus, "sources", slot, null);
  sendToWorklet({ type: "clear-bus-source", bus, slot });
}

// Gain / mute
export function setBusGain(bus: number, gain: number) {
  setBusChannels(bus, "gain", gain);
  sendToWorklet({ type: "set-bus-gain", bus, gain });
}
export function setBusMute(bus: number, muted: boolean) {
  setBusChannels(bus, "muted", muted);
  sendToWorklet({ type: "set-bus-mute", bus, muted });
}

// EQ
export function setBusEqGain(bus: number, band: number, gainDb: number) {
  setBusChannels(bus, "eqBands", band, "gainDb", gainDb);
  sendToWorklet({ type: "set-bus-eq-gain", bus, band, gainDb });
}
export function setBusEqFreq(bus: number, band: number, freqHz: number) {
  setBusChannels(bus, "eqBands", band, "freqHz", freqHz);
  sendToWorklet({ type: "set-bus-eq-freq", bus, band, freqHz });
}
export function setBusEqQ(bus: number, band: number, q: number) {
  setBusChannels(bus, "eqBands", band, "q", q);
  sendToWorklet({ type: "set-bus-eq-q", bus, band, q });
}
export function setBusEqBypass(bus: number, bypassed: boolean) {
  setBusChannels(bus, "eqBypassed", bypassed);
  sendToWorklet({ type: "set-bus-eq-bypass", bus, bypassed });
}

// Compressor
export function setBusCompEnabled(bus: number, enabled: boolean) {
  setBusChannels(bus, "compEnabled", enabled);
  sendToWorklet({ type: enabled ? "enable-bus-comp" : "disable-bus-comp", bus });
}
export function setBusCompThreshold(bus: number, v: number) { setBusChannels(bus, "compThresholdDb", v); sendToWorklet({ type: "set-bus-comp-param", bus, param: 0, value: v }); }
export function setBusCompRatio(bus: number, v: number) { setBusChannels(bus, "compRatio", v); sendToWorklet({ type: "set-bus-comp-param", bus, param: 1, value: v }); }
export function setBusCompAttack(bus: number, v: number) { setBusChannels(bus, "compAttackMs", v); sendToWorklet({ type: "set-bus-comp-param", bus, param: 2, value: v }); }
export function setBusCompRelease(bus: number, v: number) { setBusChannels(bus, "compReleaseMs", v); sendToWorklet({ type: "set-bus-comp-param", bus, param: 3, value: v }); }
export function setBusCompMakeup(bus: number, v: number) { setBusChannels(bus, "compMakeupDb", v); sendToWorklet({ type: "set-bus-comp-param", bus, param: 4, value: v }); }
export function setBusCompKnee(bus: number, v: number) { setBusChannels(bus, "compKneeDb", v); sendToWorklet({ type: "set-bus-comp-param", bus, param: 5, value: v }); }

// Gate
export function setBusGateEnabled(bus: number, enabled: boolean) { setBusChannels(bus, "gateEnabled", enabled); sendToWorklet({ type: enabled ? "enable-bus-gate" : "disable-bus-gate", bus }); }
export function setBusGateThreshold(bus: number, v: number) { setBusChannels(bus, "gateThresholdDb", v); sendToWorklet({ type: "set-bus-gate-param", bus, param: 0, value: v }); }
export function setBusGateHysteresis(bus: number, v: number) { setBusChannels(bus, "gateHysteresisDb", v); sendToWorklet({ type: "set-bus-gate-param", bus, param: 1, value: v }); }
export function setBusGateAttack(bus: number, v: number) { setBusChannels(bus, "gateAttackMs", v); sendToWorklet({ type: "set-bus-gate-param", bus, param: 2, value: v }); }
export function setBusGateRelease(bus: number, v: number) { setBusChannels(bus, "gateReleaseMs", v); sendToWorklet({ type: "set-bus-gate-param", bus, param: 3, value: v }); }
export function setBusGateHold(bus: number, v: number) { setBusChannels(bus, "gateHoldMs", v); sendToWorklet({ type: "set-bus-gate-param", bus, param: 4, value: v }); }

// Expander
export function setBusExpanderEnabled(bus: number, enabled: boolean) { setBusChannels(bus, "expanderEnabled", enabled); sendToWorklet({ type: enabled ? "enable-bus-expander" : "disable-bus-expander", bus }); }
export function setBusExpanderThreshold(bus: number, v: number) { setBusChannels(bus, "expanderThresholdDb", v); sendToWorklet({ type: "set-bus-expander-param", bus, param: 0, value: v }); }
export function setBusExpanderRatio(bus: number, v: number) { setBusChannels(bus, "expanderRatio", v); sendToWorklet({ type: "set-bus-expander-param", bus, param: 1, value: v }); }
export function setBusExpanderAttack(bus: number, v: number) { setBusChannels(bus, "expanderAttackMs", v); sendToWorklet({ type: "set-bus-expander-param", bus, param: 2, value: v }); }
export function setBusExpanderRelease(bus: number, v: number) { setBusChannels(bus, "expanderReleaseMs", v); sendToWorklet({ type: "set-bus-expander-param", bus, param: 3, value: v }); }
