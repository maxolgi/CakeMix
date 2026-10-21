import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

export const NUM_CHANNELS = 128;
export const NUM_SLOTS = 128;
export const SLOT_BASE = 128;
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
    }
  } else {
    for (let i = 0; i < channels.length; i++) {
      setChannels(i, "peakDb", -Infinity);
      setChannels(i, "rmsDb", -Infinity);
      setChannels(i, "compGrDb", 0);
      setChannels(i, "degraded", false);
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
