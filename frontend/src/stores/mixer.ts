import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

export const NUM_CHANNELS = 128;
export const NUM_SLOTS = 128;
export const SLOT_BASE = 128;
/** Console stereo channels: channel t = engine mono strips 2t (L) / 2t+1 (R). */
export const STEREO_CHANNELS = 64;
/** FX chains are pair-owned: pair t = strips 2t/2t+1, 11 slots each. */
export const FX_PAIRS = 64;
export const FX_SLOTS = 11;

export interface EqBand { gainDb: number; freqHz: number; q: number; }
export interface ChannelMeter {
  ch: number; peak: number; rms: number; gr?: number;
  /** Stream-health telemetry from the wasm FIFO layer: dg = 1 while the
   *  gate holds the strip (auto-faded to silence), sb/ins = starved
   * blocks / inserts this window, ov = cumulative overflow-dropped frames. */
  dg?: number; sb?: number; ins?: number; ov?: number;
}

export interface BusState {
  name: string;
  sources: (number | null)[];  // 16 slots
  gain: number;
  muted: boolean;
  feedsMain: boolean;
  peakDb: number;
  rmsDb: number;
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
  mainAssigned: boolean;
  peakDb: number; rmsDb: number;
  compGrDb: number;
  /** Stream-health gate active (delivery pathologically lossy — the
   *  strip is auto-faded to silence; recovers when delivery is clean). */
  degraded: boolean;
}

export interface FxParamDef {
  port: number; symbol: string; name: string;
  default: number; min: number; max: number; step: number; integer: boolean;
}

export interface FxEffectDef {
  id: number; key: string; label: string; category: string; stereo: boolean;
  /** DSP factory live in the engine build. False = catalog row whose TU
   *  has not been ported yet (the engine rejects it at set-fx-slot). */
  available: boolean;
  params: FxParamDef[];
}

export interface FxCatalog {
  slots: number; slotParams: number; effects: FxEffectDef[];
}

/** One chain slot: catalog effect id (-1 = empty), enabled flag, and
 *  the LIVE param row in ENGINE units — trimmed to the effect's param
 *  count (the 48-wide padded row is a wasm storage artifact). */
export interface FxSlotState { fx: number; enabled: boolean; params: number[]; }

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
    mainAssigned: true,
    peakDb: -Infinity, rmsDb: -Infinity,
    compGrDb: 0,
    degraded: false,
  };
}

export const [channels, setChannels] = createStore<ChannelState[]>(
  Array.from({ length: NUM_CHANNELS + NUM_SLOTS }, (_, i) => defaultChannel(i))
);

// ── Stereo console model (64 channels = engine pairs 2t/2t+1) ───────────────
//
// The console UI speaks in STEREO CHANNELS: stereo channel t maps to engine
// mono channels 2t (L) and 2t+1 (R) — every control write fans out to both
// (the OctOBX mixer-store pattern over the same engine). The mono channel
// store above stays the engine-facing mirror AND the bus-slot model: every
// stereo setter also writes the two mono entries, so mono readers (the bus
// source dropdown, EQCurve, scene pulls) never see stale values.

/**
 * Balance → per-side pan wiring. Feeding both sides at pan=0 (center)
 * makes master_L = master_R = 0.5·L + 0.5·R — a collapsed mono sum.
 * Instead the L channel pans HARD LEFT and the R channel HARD RIGHT
 * (unity passthrough, true stereo preserved), and the stereo channel's
 * balance offsets both sides: at balance −1 both pans sit hard left, at
 * +1 hard right. Inverse property used by scene recall: lPan + rPan =
 * 2·balance.
 */
export function channelPans(balance: number): { lPan: number; rPan: number } {
  const b = Math.max(-1, Math.min(1, balance));
  return {
    lPan: -1 + 2 * Math.max(0, b),
    rPan: 1 + 2 * Math.min(0, b),
  };
}

export interface StereoMeters {
  peakL: number; rmsL: number; peakR: number; rmsR: number;
  /** Compressor gain reduction per side, dB (0 = none). */
  grL: number; grR: number;
  /** Stream-health gate active on either side (see ChannelState.degraded). */
  degraded: boolean;
}

/** Stereo console channel: every control field fans out to strips 2t/2t+1. */
export interface StereoChannelState extends Omit<
  ChannelState, "name" | "pan" | "peakDb" | "rmsDb" | "compGrDb" | "degraded"
> {
  name: string;
  /** -1..1 balance across the L/R pair (see channelPans). */
  balance: number;
  /** Per-side stream presence, fed from the worklet's pid-mapped events. */
  sideMapped: { l: boolean; r: boolean };
  meters: StereoMeters;
}

function defaultStereoChannel(index: number): StereoChannelState {
  // Defaults mirror the per-channel defaults (strip 2t's); the mono-only
  // fields are dropped — meters/sideMapped are runtime state, not params.
  const { pan, peakDb, rmsDb, compGrDb, degraded, ...params } = defaultChannel(index * 2);
  return {
    ...params,
    name: `Ch ${index + 1}`,
    balance: 0,
    sideMapped: { l: false, r: false },
    meters: {
      peakL: -Infinity, rmsL: -Infinity,
      peakR: -Infinity, rmsR: -Infinity,
      grL: 0, grR: 0, degraded: false,
    },
  };
}

export const [stereoChannels, setStereoChannels] = createStore<StereoChannelState[]>(
  Array.from({ length: STEREO_CHANNELS }, (_, i) => defaultStereoChannel(i))
);

function defaultBus(index: number): BusState {
  return {
    name: `Bus ${index + 1}`,
    sources: Array.from({ length: 16 }, () => null),
    gain: 1.0, muted: false, feedsMain: true,
    peakDb: -Infinity, rmsDb: -Infinity,
  };
}

export const [busChannels, setBusChannels] = createStore<BusState[]>(
  Array.from({ length: 8 }, (_, i) => defaultBus(i))
);
export const [selectedBus, setSelectedBus] = createSignal<number | null>(null);

function defaultFxSlot(): FxSlotState { return { fx: -1, enabled: false, params: [] }; }

/** Per-pair FX chains: fxChains[pair][slot]. pair = ch/2 for the even
 *  strip ch owning the stereo pair (see the fx* helpers below). */
export const [fxChains, setFxChains] = createStore<FxSlotState[][]>(
  Array.from({ length: FX_PAIRS }, () => Array.from({ length: FX_SLOTS }, defaultFxSlot))
);
/** The worklet's `fx-catalog` reply, parsed (null until loaded). */
export const [fxCatalog, setFxCatalogState] = createSignal<FxCatalog | null>(null);

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
export function sendToWorklet(msg: any, transfer: Transferable[] = []) { if (mixerNode) mixerNode.port.postMessage(msg, transfer); }

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
  // Empty channel/bus lists (worklet's stopped report) reset every meter to
  // -Infinity so the UI falls to zero; otherwise only reported entries update.
  if (msg.channels && msg.channels.length) {
    for (const cm of msg.channels) {
      setChannels(cm.ch, "peakDb", cm.peak);
      setChannels(cm.ch, "rmsDb", cm.rms);
      setChannels(cm.ch, "compGrDb", cm.gr ?? 0);
      setChannels(cm.ch, "degraded", cm.dg === 1);
      // Input strips additionally fold into the stereo store: even ch → L,
      // odd → R of stereo channel ch>>1. Bus-slot strips (>= SLOT_BASE)
      // have no stereo pair and only take the mono writes above.
      if (cm.ch < NUM_CHANNELS) {
        const t = cm.ch >> 1;
        if (cm.ch & 1) {
          setStereoChannels(t, "meters", "peakR", cm.peak);
          setStereoChannels(t, "meters", "rmsR", cm.rms);
          setStereoChannels(t, "meters", "grR", cm.gr ?? 0);
        } else {
          setStereoChannels(t, "meters", "peakL", cm.peak);
          setStereoChannels(t, "meters", "rmsL", cm.rms);
          setStereoChannels(t, "meters", "grL", cm.gr ?? 0);
        }
        // Degraded = stream-health gate on EITHER side. The other side's
        // mono entry was updated earlier in this batch (or holds the last
        // known state), so reading it back gives the correct OR.
        setStereoChannels(t, "meters", "degraded", cm.dg === 1 || channels[cm.ch ^ 1].degraded);
      }
    }
  } else {
    for (let i = 0; i < channels.length; i++) {
      setChannels(i, "peakDb", -Infinity);
      setChannels(i, "rmsDb", -Infinity);
      setChannels(i, "compGrDb", 0);
      setChannels(i, "degraded", false);
    }
    for (let t = 0; t < STEREO_CHANNELS; t++) {
      setStereoChannels(t, "meters", "peakL", -Infinity);
      setStereoChannels(t, "meters", "rmsL", -Infinity);
      setStereoChannels(t, "meters", "peakR", -Infinity);
      setStereoChannels(t, "meters", "rmsR", -Infinity);
      setStereoChannels(t, "meters", "grL", 0);
      setStereoChannels(t, "meters", "grR", 0);
      setStereoChannels(t, "meters", "degraded", false);
    }
  }
  if (msg.buses && msg.buses.length) {
    for (const bm of msg.buses) {
      setBusChannels(bm.bus, "peakDb", bm.peak);
      setBusChannels(bm.bus, "rmsDb", bm.rms);
    }
  } else {
    for (let i = 0; i < busChannels.length; i++) {
      setBusChannels(i, "peakDb", -Infinity);
      setBusChannels(i, "rmsDb", -Infinity);
    }
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
export function setMainAssign(ch: number, on: boolean) { setChannels(ch, "mainAssigned", on); sendToWorklet({ type: "set-main-assign", ch, on }); }
export function setMasterGain(gain: number) { setMasterGainState(gain); sendToWorklet({ type: "set-master-gain", gain }); }
export function setLimiterEnabled(enabled: boolean) { setLimiterEnabledState(enabled); sendToWorklet({ type: "set-limiter", enabled }); }
export function setLimiterCeiling(ceilingDb: number) { setLimiterCeilingState(ceilingDb); sendToWorklet({ type: "set-limiter-ceiling", ceilingDb }); }
export function setLimiterRelease(releaseMs: number) { setLimiterReleaseState(releaseMs); sendToWorklet({ type: "set-limiter-release", releaseMs }); }
export function clearMasterClip() { sendToWorklet({ type: "clear-clip" }); }

// ── Stereo console helpers ───────────────────────────────────────────────────
//
// Stereo version of every mono control helper above: writes the
// StereoChannel store, mirrors both mono entries, and posts the SAME
// message (identical field names) to BOTH engine channels 2t / 2t+1.
// The mono helpers stay exported and unchanged — bus slots (strips
// 128-255) still use them.

function stereoSend(t: number, msg: (ch: number) => object): void {
  sendToWorklet(msg(2 * t));
  sendToWorklet(msg(2 * t + 1));
}

export function setStereoName(t: number, name: string) {
  setStereoChannels(t, "name", name);
  setChannels(2 * t, "name", name);
  setChannels(2 * t + 1, "name", name);
  stereoSend(t, ch => ({ type: "set-name", ch, name }));
}
export function setStereoGain(t: number, gain: number) {
  setStereoChannels(t, "gain", gain);
  setChannels(2 * t, "gain", gain);
  setChannels(2 * t + 1, "gain", gain);
  stereoSend(t, ch => ({ type: "set-gain", ch, gain }));
}
export function setStereoBalance(t: number, balance: number) {
  setStereoChannels(t, "balance", balance);
  // Balance → per-side pans (see channelPans); each engine channel gets
  // its own pan, and the mono mirror keeps each side's actual pan.
  const { lPan, rPan } = channelPans(balance);
  setChannels(2 * t, "pan", lPan);
  setChannels(2 * t + 1, "pan", rPan);
  sendToWorklet({ type: "set-pan", ch: 2 * t, pan: lPan });
  sendToWorklet({ type: "set-pan", ch: 2 * t + 1, pan: rPan });
}
export function setStereoInputGain(t: number, gainDb: number) {
  setStereoChannels(t, "inputGainDb", gainDb);
  setChannels(2 * t, "inputGainDb", gainDb);
  setChannels(2 * t + 1, "inputGainDb", gainDb);
  stereoSend(t, ch => ({ type: "set-input-gain", ch, gainDb }));
}
export function setStereoPhase(t: number, inverted: boolean) {
  setStereoChannels(t, "phaseInverted", inverted);
  setChannels(2 * t, "phaseInverted", inverted);
  setChannels(2 * t + 1, "phaseInverted", inverted);
  stereoSend(t, ch => ({ type: "set-phase", ch, inverted }));
}
export function setStereoPanLaw(t: number, law: number) {
  setStereoChannels(t, "panLaw", law);
  setChannels(2 * t, "panLaw", law);
  setChannels(2 * t + 1, "panLaw", law);
  stereoSend(t, ch => ({ type: "set-pan-law", ch, law }));
}
export function setStereoMute(t: number, muted: boolean) {
  setStereoChannels(t, "muted", muted);
  setChannels(2 * t, "muted", muted);
  setChannels(2 * t + 1, "muted", muted);
  stereoSend(t, ch => ({ type: "set-mute", ch, muted }));
}
export function setStereoSolo(t: number, soloed: boolean) {
  setStereoChannels(t, "soloed", soloed);
  setChannels(2 * t, "soloed", soloed);
  setChannels(2 * t + 1, "soloed", soloed);
  stereoSend(t, ch => ({ type: "set-solo", ch, soloed }));
}
export function setStereoMainAssign(t: number, on: boolean) {
  setStereoChannels(t, "mainAssigned", on);
  setChannels(2 * t, "mainAssigned", on);
  setChannels(2 * t + 1, "mainAssigned", on);
  stereoSend(t, ch => ({ type: "set-main-assign", ch, on }));
}
export function setStereoEqGain(t: number, band: number, gainDb: number) {
  setStereoChannels(t, "eqBands", band, "gainDb", gainDb);
  setChannels(2 * t, "eqBands", band, "gainDb", gainDb);
  setChannels(2 * t + 1, "eqBands", band, "gainDb", gainDb);
  stereoSend(t, ch => ({ type: "set-eq-gain", ch, band, gainDb }));
}
export function setStereoEqFreq(t: number, band: number, freqHz: number) {
  setStereoChannels(t, "eqBands", band, "freqHz", freqHz);
  setChannels(2 * t, "eqBands", band, "freqHz", freqHz);
  setChannels(2 * t + 1, "eqBands", band, "freqHz", freqHz);
  stereoSend(t, ch => ({ type: "set-eq-freq", ch, band, freqHz }));
}
export function setStereoEqQ(t: number, band: number, q: number) {
  setStereoChannels(t, "eqBands", band, "q", q);
  setChannels(2 * t, "eqBands", band, "q", q);
  setChannels(2 * t + 1, "eqBands", band, "q", q);
  stereoSend(t, ch => ({ type: "set-eq-q", ch, band, q }));
}
export function setStereoEqBypass(t: number, bypassed: boolean) {
  setStereoChannels(t, "eqBypassed", bypassed);
  setChannels(2 * t, "eqBypassed", bypassed);
  setChannels(2 * t + 1, "eqBypassed", bypassed);
  stereoSend(t, ch => ({ type: "set-eq-bypass", ch, bypassed }));
}

// Dynamics: enable recreates the engine effect with ITS OWN defaults, so
// every enable is followed by a push of all displayed params (the OctOBX
// pushCompParams/pushGateParams/pushExpParams discipline).

export function setStereoCompEnabled(t: number, enabled: boolean) {
  setStereoChannels(t, "compEnabled", enabled);
  setChannels(2 * t, "compEnabled", enabled);
  setChannels(2 * t + 1, "compEnabled", enabled);
  stereoSend(t, ch => ({ type: enabled ? "enable-compressor" : "disable-compressor", ch }));
  if (enabled) pushStereoCompParams(t);
}
/** Re-send all displayed comp params to both sides. */
export function pushStereoCompParams(t: number) {
  const s = stereoChannels[t];
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 0, value: s.compThresholdDb }));
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 1, value: s.compRatio }));
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 2, value: s.compAttackMs }));
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 3, value: s.compReleaseMs }));
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 4, value: s.compMakeupDb }));
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 5, value: s.compKneeDb }));
}
export function setStereoCompThreshold(t: number, v: number) {
  setStereoChannels(t, "compThresholdDb", v);
  setChannels(2 * t, "compThresholdDb", v);
  setChannels(2 * t + 1, "compThresholdDb", v);
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 0, value: v }));
}
export function setStereoCompRatio(t: number, v: number) {
  setStereoChannels(t, "compRatio", v);
  setChannels(2 * t, "compRatio", v);
  setChannels(2 * t + 1, "compRatio", v);
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 1, value: v }));
}
export function setStereoCompAttack(t: number, v: number) {
  setStereoChannels(t, "compAttackMs", v);
  setChannels(2 * t, "compAttackMs", v);
  setChannels(2 * t + 1, "compAttackMs", v);
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 2, value: v }));
}
export function setStereoCompRelease(t: number, v: number) {
  setStereoChannels(t, "compReleaseMs", v);
  setChannels(2 * t, "compReleaseMs", v);
  setChannels(2 * t + 1, "compReleaseMs", v);
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 3, value: v }));
}
export function setStereoCompMakeup(t: number, v: number) {
  setStereoChannels(t, "compMakeupDb", v);
  setChannels(2 * t, "compMakeupDb", v);
  setChannels(2 * t + 1, "compMakeupDb", v);
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 4, value: v }));
}
export function setStereoCompKnee(t: number, v: number) {
  setStereoChannels(t, "compKneeDb", v);
  setChannels(2 * t, "compKneeDb", v);
  setChannels(2 * t + 1, "compKneeDb", v);
  stereoSend(t, ch => ({ type: "set-comp-param", ch, param: 5, value: v }));
}

export function setStereoGateEnabled(t: number, enabled: boolean) {
  setStereoChannels(t, "gateEnabled", enabled);
  setChannels(2 * t, "gateEnabled", enabled);
  setChannels(2 * t + 1, "gateEnabled", enabled);
  stereoSend(t, ch => ({ type: enabled ? "enable-gate" : "disable-gate", ch }));
  if (enabled) pushStereoGateParams(t);
}
/** Re-send all displayed gate params to both sides (see pushStereoCompParams). */
export function pushStereoGateParams(t: number) {
  const s = stereoChannels[t];
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 0, value: s.gateThresholdDb }));
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 1, value: s.gateHysteresisDb }));
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 2, value: s.gateAttackMs }));
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 3, value: s.gateReleaseMs }));
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 4, value: s.gateHoldMs }));
}
export function setStereoGateThreshold(t: number, v: number) {
  setStereoChannels(t, "gateThresholdDb", v);
  setChannels(2 * t, "gateThresholdDb", v);
  setChannels(2 * t + 1, "gateThresholdDb", v);
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 0, value: v }));
}
export function setStereoGateHysteresis(t: number, v: number) {
  setStereoChannels(t, "gateHysteresisDb", v);
  setChannels(2 * t, "gateHysteresisDb", v);
  setChannels(2 * t + 1, "gateHysteresisDb", v);
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 1, value: v }));
}
export function setStereoGateAttack(t: number, v: number) {
  setStereoChannels(t, "gateAttackMs", v);
  setChannels(2 * t, "gateAttackMs", v);
  setChannels(2 * t + 1, "gateAttackMs", v);
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 2, value: v }));
}
export function setStereoGateRelease(t: number, v: number) {
  setStereoChannels(t, "gateReleaseMs", v);
  setChannels(2 * t, "gateReleaseMs", v);
  setChannels(2 * t + 1, "gateReleaseMs", v);
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 3, value: v }));
}
export function setStereoGateHold(t: number, v: number) {
  setStereoChannels(t, "gateHoldMs", v);
  setChannels(2 * t, "gateHoldMs", v);
  setChannels(2 * t + 1, "gateHoldMs", v);
  stereoSend(t, ch => ({ type: "set-gate-param", ch, param: 4, value: v }));
}

export function setStereoExpanderEnabled(t: number, enabled: boolean) {
  setStereoChannels(t, "expanderEnabled", enabled);
  setChannels(2 * t, "expanderEnabled", enabled);
  setChannels(2 * t + 1, "expanderEnabled", enabled);
  stereoSend(t, ch => ({ type: enabled ? "enable-expander" : "disable-expander", ch }));
  if (enabled) pushStereoExpParams(t);
}
/** Re-send all displayed expander params to both sides. */
export function pushStereoExpParams(t: number) {
  const s = stereoChannels[t];
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 0, value: s.expanderThresholdDb }));
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 1, value: s.expanderRatio }));
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 2, value: s.expanderAttackMs }));
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 3, value: s.expanderReleaseMs }));
}
export function setStereoExpanderThreshold(t: number, v: number) {
  setStereoChannels(t, "expanderThresholdDb", v);
  setChannels(2 * t, "expanderThresholdDb", v);
  setChannels(2 * t + 1, "expanderThresholdDb", v);
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 0, value: v }));
}
export function setStereoExpanderRatio(t: number, v: number) {
  setStereoChannels(t, "expanderRatio", v);
  setChannels(2 * t, "expanderRatio", v);
  setChannels(2 * t + 1, "expanderRatio", v);
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 1, value: v }));
}
export function setStereoExpanderAttack(t: number, v: number) {
  setStereoChannels(t, "expanderAttackMs", v);
  setChannels(2 * t, "expanderAttackMs", v);
  setChannels(2 * t + 1, "expanderAttackMs", v);
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 2, value: v }));
}
export function setStereoExpanderRelease(t: number, v: number) {
  setStereoChannels(t, "expanderReleaseMs", v);
  setChannels(2 * t, "expanderReleaseMs", v);
  setChannels(2 * t + 1, "expanderReleaseMs", v);
  stereoSend(t, ch => ({ type: "set-exp-param", ch, param: 3, value: v }));
}

// ── Stereo side-mapped tracking + FX anchor ──────────────────────────────────

/**
 * Fold one side's stream presence into sideMapped: mono engine ch even →
 * L, odd → R of stereo channel ch>>1. Called from App's router on every
 * worklet `pid-mapped` event (chStart >= 0 ⇒ mapped = true for
 * [chStart, chStart + channelCount)).
 *
 * NOTE: there is no unmap echo today — unmapping happens in
 * quicsrt/store.ts `unmapSessionPids`, which posts `unmap-pid` worklet
 * messages without notifying the mixer stores. A disconnected session's
 * sides therefore stay marked (until reload); the worklet's channel
 * allocator never reuses ranges, so the stale flag only affects the
 * derived `stereoExists`. Call applySideMapped(ch, false) there when an
 * unmap event is added.
 */
export function applySideMapped(ch: number, mapped: boolean) {
  if (ch < 0 || ch >= NUM_CHANNELS) return;  // bus-slot strips have no pair
  setStereoChannels(ch >> 1, "sideMapped", ch & 1 ? "r" : "l", mapped);
}

/** True when either side of stereo channel t carries a mapped stream. */
export function stereoExists(t: number): boolean {
  const s = stereoChannels[t];
  return !!s && (s.sideMapped.l || s.sideMapped.r);
}

/** The EVEN engine strip (2t) that owns stereo channel t's FX chain —
 *  the ch every set-fx-* worklet message expects (FX chains are
 *  pair-owned; pair t = strips 2t/2t+1). */
export function fxAnchor(t: number): number {
  return 2 * t;
}

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

// Feeds-master toggle: off = independent bus mix (monitor/N-1)
export function setBusFeedsMain(bus: number, on: boolean) {
  setBusChannels(bus, "feedsMain", on);
  sendToWorklet({ type: "set-bus-feeds-main", bus, on });
}

// Slots are full channel strips at indices 128-255.
// Slot idx = SLOT_BASE + bus*16 + slot; use the channel helpers
// (setEqGain, setCompEnabled, …) with this index.
export function slotChannelIndex(bus: number, slot: number): number {
  return SLOT_BASE + bus * 16 + slot;
}

// ── FX chain helpers ──────────────────────────────────────────────────────────

/** Pair index of even strip `ch`, or null when ch is not an even input
 *  strip (0-126). All fx helpers take the EVEN strip of a stereo pair —
 *  exactly the ch the worklet's set-fx-* messages expect. */
function fxPairOf(ch: number): number | null {
  return ch >= 0 && ch < NUM_CHANNELS && ch % 2 === 0 ? ch / 2 : null;
}

/** Store the worklet's `fx-catalog` reply (the App router passes the
 *  reply's json string, same flow as applyConsoleParams). Parses
 *  defensively: a malformed payload leaves the catalog untouched. */
export function applyFxCatalog(json: string) {
  try {
    const c = JSON.parse(json);
    if (c && typeof c === "object" && Array.isArray(c.effects)) setFxCatalogState(c);
  } catch { /* malformed payload: catalog stays as-is */ }
}

/** Ask the worklet for the FX catalog (request/reply like get-params);
 *  its `fx-catalog` reply routes to `applyFxCatalog`. */
export function requestFxCatalog() {
  sendToWorklet({ type: "get-fx-catalog" });
}

/** An engine fx-op rejection relayed by the worklet (`fx-error`): which
 *  op on which channel:slot failed, and the engine's (already descriptive)
 *  message. */
export interface FxErrorPayload {
  op: "set-fx-slot" | "set-fx-enabled" | "set-fx-param" | "move-fx-slot";
  ch: number; slot: number; msg: string;
}

/** Latest fx rejection, null when none — transient: App's toast surfaces
 *  it and auto-clears after ~4s. */
export const [fxLastError, setFxLastError] = createSignal<FxErrorPayload | null>(null);

/** Pre-op snapshot of one in-flight fx mutation, keyed `${ch}:${slot}`
 *  (ch = even pair-owner strip; slot = the slot the worklet names in its
 *  fx-error reply — the FROM slot for moves). Per-slot ops snapshot just
 *  the slot; a move snapshots the whole pair row, because the error reply
 *  carries no `to` — rollback must be derivable from ch:slot alone. */
type FxSnapshot = { kind: "slot"; state: FxSlotState } | { kind: "row"; states: FxSlotState[] };
const fxPending = new Map<string, FxSnapshot>();

function snapshotFxSlot(pair: number, slot: number): FxSlotState {
  const s = fxChains[pair][slot];
  return { fx: s.fx, enabled: s.enabled, params: [...s.params] };
}

function snapshotFxRow(pair: number): FxSlotState[] {
  return fxChains[pair].map(s => ({ fx: s.fx, enabled: s.enabled, params: [...s.params] }));
}

/** Reconcile a rejected fx op (the worklet's `fx-error` reply, routed by
 *  App): roll the slot — or the whole pair row, for moves — back to the
 *  snapshot taken when the op was posted, drop the snapshot, and surface
 *  the engine's message via `fxLastError`. The store is an optimistic
 *  cache; this is what keeps it from showing state the engine refused.
 *
 *  Superseded ops: a NEW user op on the same ch:slot replaces the stored
 *  snapshot, so at most one rollback state exists per slot — a late error
 *  from the superseded op cannot restore the older pre-state; it consumes
 *  the newest snapshot for that slot instead (and still toasts). Errors
 *  with no snapshot at all (none in flight) only toast. */
export function applyFxError(payload: FxErrorPayload) {
  const pair = fxPairOf(payload.ch);
  const key = `${payload.ch}:${payload.slot}`;
  const snap = fxPending.get(key);
  if (snap && pair !== null && payload.slot >= 0 && payload.slot < FX_SLOTS) {
    if (snap.kind === "row") {
      setFxChains(pair, snap.states.map(s => ({ fx: s.fx, enabled: s.enabled, params: [...s.params] })));
    } else {
      setFxChains(pair, payload.slot, { fx: snap.state.fx, enabled: snap.state.enabled, params: [...snap.state.params] });
    }
    fxPending.delete(key);
  }
  setFxLastError({ op: payload.op, ch: payload.ch, slot: payload.slot, msg: payload.msg });
}

/** The worklet's `fx-out` reply (routed by App): the 8 control OUTPUT
 *  floats of one (ch, slot). Display-only — the tuner readout polls it
 *  at a human rate; the looper's meter rows have no UI yet. Latest
 *  reply wins: readers match on (ch, slot) and pollers re-request, so a
 *  single slot of state is all the UI needs. */
export interface FxOutPayload { ch: number; slot: number; values: number[]; }
export const [fxOut, setFxOut] = createSignal<FxOutPayload | null>(null);
export function applyFxOut(payload: FxOutPayload) { setFxOut(payload); }

// Store helpers — each updates store + sends worklet message. The wasm
// is the source of truth; these mirror its exact semantics (param is
// the catalog-row ORDINAL in ENGINE units, params rows live-trimmed) so
// store and engine never disagree. Guards no-op WITHOUT posting — the
// wasm rejects/ignores those calls and changes nothing engine-side. The
// engine can still reject a POSTED op (delay/echo/reverb instance cap,
// not-yet-available effects), so each helper snapshots the affected
// pre-op state into fxPending first; a rejection's fx-error reply rolls
// it back via applyFxError above.
export function setFxSlot(ch: number, slot: number, fxId: number) {
  const pair = fxPairOf(ch);
  if (pair === null || slot < 0 || slot >= FX_SLOTS) return;
  // -1 empties the slot; any other id must resolve in the catalog AND be
  // available (the wasm rejects ids outside it and null-factory rows —
  // and without a catalog we cannot know the defaults it would install,
  // so don't post either).
  const fx = fxId >= 0 ? fxCatalog()?.effects[fxId] : undefined;
  if (fxId !== -1 && (!fx || !fx.available)) return;
  fxPending.set(`${ch}:${slot}`, { kind: "slot", state: snapshotFxSlot(pair, slot) });
  setFxChains(pair, slot, { fx: fxId, enabled: false, params: fx ? fx.params.map(p => p.default) : [] });
  sendToWorklet({ type: "set-fx-slot", ch, slot, fxId });
}
export function setFxEnabled(ch: number, slot: number, on: boolean) {
  const pair = fxPairOf(ch);
  if (pair === null || slot < 0 || slot >= FX_SLOTS) return;
  if (fxChains[pair][slot].fx < 0) return;  // wasm ignores empty slots
  fxPending.set(`${ch}:${slot}`, { kind: "slot", state: snapshotFxSlot(pair, slot) });
  setFxChains(pair, slot, "enabled", on);
  sendToWorklet({ type: "set-fx-enabled", ch, slot, on });
}
export function setFxParam(ch: number, slot: number, param: number, value: number) {
  const pair = fxPairOf(ch);
  if (pair === null || slot < 0 || slot >= FX_SLOTS) return;
  if (param < 0 || param >= fxChains[pair][slot].params.length) return;
  fxPending.set(`${ch}:${slot}`, { kind: "slot", state: snapshotFxSlot(pair, slot) });
  setFxChains(pair, slot, "params", param, value);
  sendToWorklet({ type: "set-fx-param", ch, slot, param, value });
}
export function moveFxSlot(ch: number, from: number, to: number) {
  const pair = fxPairOf(ch);
  if (pair === null) return;
  if (from < 0 || from >= FX_SLOTS || to < 0 || to >= FX_SLOTS) return;
  // Whole-row snapshot under the from-slot key (fx-error replies name
  // only the from slot); the array-move below must restore exactly.
  fxPending.set(`${ch}:${from}`, { kind: "row", states: snapshotFxRow(pair) });
  // Array-move the whole row (fx id, params, enabled travel together),
  // same semantics as the wasm's fx_move_slot.
  const rows = snapshotFxRow(pair);
  const [row] = rows.splice(from, 1);
  rows.splice(to, 0, row);
  setFxChains(pair, rows);
  sendToWorklet({ type: "move-fx-slot", ch, from, to });
}

// ── Scene recall apply ──────────────────────────────────────────────────────

/** Apply a console-params document (the worklet's `console-params` reply,
 *  pulled after `scene-recalled`) to the UI stores so every control
 *  reflects the recalled console. Field names mirror `ChannelState` /
 *  `BusState` exactly — the wasm serializes them in lockstep. `fxChains`
 *  syncs the FX pair store the same way.
 *
 *  Writes the stores DIRECTLY, never through the sendToWorklet helpers
 *  above: the mixer already holds these values, and echoing them back
 *  would cancel an in-progress scene cross-fade (every wasm setter is
 *  cancel-on-set). Meters are not part of the document and stay
 *  untouched. Runs at recall rate (user clicks), so a full-console
 *  parse per call is fine. */
export function applyConsoleParams(json: string) {
  const p = JSON.parse(json);
  for (const s of p.strips) {
    const ch = s.ch;
    setChannels(ch, "name", s.name);
    setChannels(ch, "gain", s.gain);
    setChannels(ch, "pan", s.pan);
    setChannels(ch, "inputGainDb", s.inputGainDb);
    setChannels(ch, "phaseInverted", s.phaseInverted);
    setChannels(ch, "panLaw", s.panLaw);
    setChannels(ch, "muted", s.muted);
    setChannels(ch, "soloed", s.soloed);
    setChannels(ch, "mainAssigned", s.mainAssigned);
    setChannels(ch, "eqBypassed", s.eqBypassed);
    for (let b = 0; b < s.eqBands.length; b++) {
      setChannels(ch, "eqBands", b, "gainDb", s.eqBands[b].gainDb);
      setChannels(ch, "eqBands", b, "freqHz", s.eqBands[b].freqHz);
      setChannels(ch, "eqBands", b, "q", s.eqBands[b].q);
    }
    setChannels(ch, "compEnabled", s.compEnabled);
    setChannels(ch, "compThresholdDb", s.compThresholdDb);
    setChannels(ch, "compRatio", s.compRatio);
    setChannels(ch, "compKneeDb", s.compKneeDb);
    setChannels(ch, "compAttackMs", s.compAttackMs);
    setChannels(ch, "compReleaseMs", s.compReleaseMs);
    setChannels(ch, "compMakeupDb", s.compMakeupDb);
    setChannels(ch, "gateEnabled", s.gateEnabled);
    setChannels(ch, "gateThresholdDb", s.gateThresholdDb);
    setChannels(ch, "gateHysteresisDb", s.gateHysteresisDb);
    setChannels(ch, "gateAttackMs", s.gateAttackMs);
    setChannels(ch, "gateReleaseMs", s.gateReleaseMs);
    setChannels(ch, "gateHoldMs", s.gateHoldMs);
    setChannels(ch, "expanderEnabled", s.expanderEnabled);
    setChannels(ch, "expanderThresholdDb", s.expanderThresholdDb);
    setChannels(ch, "expanderRatio", s.expanderRatio);
    setChannels(ch, "expanderAttackMs", s.expanderAttackMs);
    setChannels(ch, "expanderReleaseMs", s.expanderReleaseMs);
  }
  // Stereo derivation: stereo channel t = strips 2t/2t+1 as just written
  // above, L wins on mismatch (the fan-out setters wrote both sides
  // identically, so mismatches only exist on legacy mono-per-strip
  // scenes). Balance is recovered from the per-side pans — channelPans
  // guarantees lPan + rPan = 2·balance (clamped for arbitrary legacy
  // values). Same write-directly-never-echo rule; the object write is a
  // shallow merge, so meters/sideMapped (runtime state, not part of the
  // document) stay untouched.
  for (let t = 0; t < STEREO_CHANNELS; t++) {
    const l = channels[2 * t];
    const r = channels[2 * t + 1];
    setStereoChannels(t, {
      name: l.name,
      gain: l.gain,
      balance: Math.max(-1, Math.min(1, (l.pan + r.pan) / 2)),
      inputGainDb: l.inputGainDb,
      phaseInverted: l.phaseInverted,
      panLaw: l.panLaw,
      muted: l.muted,
      soloed: l.soloed,
      mainAssigned: l.mainAssigned,
      eqBypassed: l.eqBypassed,
      eqBands: l.eqBands.map(b => ({ ...b })),
      gateEnabled: l.gateEnabled,
      gateThresholdDb: l.gateThresholdDb,
      gateHysteresisDb: l.gateHysteresisDb,
      gateAttackMs: l.gateAttackMs,
      gateReleaseMs: l.gateReleaseMs,
      gateHoldMs: l.gateHoldMs,
      compEnabled: l.compEnabled,
      compThresholdDb: l.compThresholdDb,
      compRatio: l.compRatio,
      compKneeDb: l.compKneeDb,
      compAttackMs: l.compAttackMs,
      compReleaseMs: l.compReleaseMs,
      compMakeupDb: l.compMakeupDb,
      expanderEnabled: l.expanderEnabled,
      expanderThresholdDb: l.expanderThresholdDb,
      expanderRatio: l.expanderRatio,
      expanderAttackMs: l.expanderAttackMs,
      expanderReleaseMs: l.expanderReleaseMs,
    });
  }
  for (let b = 0; b < p.buses.length; b++) {
    const bus = p.buses[b];
    setBusChannels(b, "gain", bus.gain);
    setBusChannels(b, "muted", bus.muted);
    setBusChannels(b, "feedsMain", bus.feedsMain);
    for (let slot = 0; slot < bus.sources.length; slot++) {
      setBusChannels(b, "sources", slot, bus.sources[slot]);
    }
  }
  // FX chains: fxChains carries NON-EMPTY pairs only — an absent pair is
  // an empty chain (reset to default). Entries hold all 11 slots; a
  // set-but-disabled slot still carries fx + params (a recall sync must
  // not lose them). Same write-directly-never-echo rule as above.
  const fxDocs: (any | undefined)[] = Array.from({ length: FX_PAIRS }, () => undefined);
  for (const c of p.fxChains ?? []) fxDocs[c.pair] = c;
  for (let pair = 0; pair < FX_PAIRS; pair++) {
    const slots = fxDocs[pair]?.slots ?? [];
    for (let slot = 0; slot < FX_SLOTS; slot++) {
      const s = slots[slot];
      setFxChains(pair, slot, s && s.fx >= 0
        ? { fx: s.fx, enabled: !!s.enabled, params: s.params ?? [] }
        : defaultFxSlot());
    }
  }
  setMasterGainState(p.masterGain);
}
