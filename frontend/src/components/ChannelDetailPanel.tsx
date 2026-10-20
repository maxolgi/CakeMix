import { createSignal } from "solid-js";
import {
  channels,
  setInputGain, setPhase, setPanLaw, setChannelName,
  setEqGain, setEqFreq, setEqQ, setEqBypass,
  setCompEnabled, setCompThreshold, setCompRatio, setCompAttack, setCompRelease, setCompKnee, setCompMakeup,
  setGateEnabled, setGateThreshold, setGateHysteresis, setGateAttack, setGateRelease, setGateHold,
  setExpanderEnabled, setExpanderThreshold, setExpanderRatio, setExpanderAttack, setExpanderRelease,
  setMainAssign,
  busChannels, setBusSource, clearBusSource, NUM_CHANNELS, FX_SLOTS,
  fxChains, fxCatalog, setFxSlot, setFxEnabled, setFxParam, moveFxSlot, type FxParamDef,
  faderToGain, gainToFader, formatGainDb,
  setChannels, sendToWorklet,
} from "../stores/mixer";
import { Knob } from "./Knob";
import { GrMeter } from "./GrMeter";
import { MeterCanvas } from "./MeterCanvas";
import { EQCurve } from "./EQCurve";

// 6-band EQ layout. Band 0 (HPF) and band 5 (High Shelf) have no gain control.
// Defaults mirror stores/mixer.ts EQ_BAND_DEFAULTS.
const EQ_BANDS = [
  { name: "HPF", freq: 80, q: 0.707, hasGain: false },
  { name: "LOW", freq: 120, q: 0.707, hasGain: true },
  { name: "L-MID", freq: 400, q: 1.0, hasGain: true },
  { name: "MID", freq: 1500, q: 1.0, hasGain: true },
  { name: "H-MID", freq: 5000, q: 1.0, hasGain: true },
  { name: "HIGH", freq: 10000, q: 0.707, hasGain: false },
];

const PAN_LAWS = ["Linear", "-3dB", "-4.5dB", "-6dB"];

const fmtDb = (v: number) => v.toFixed(1);
const fmtRatio = (v: number) => (v >= 20 ? "20:1" : v.toFixed(1) + ":1");
const fmtHz = (v: number) => (v >= 1000 ? (v / 1000).toFixed(1) + "k" : v.toFixed(0));
const fmtMs = (v: number) => (v < 1 ? v.toFixed(2) : v < 10 ? v.toFixed(1) : v.toFixed(0));
const fmtFx = (v: number) => (Math.abs(v) >= 100 ? v.toFixed(0) : Math.abs(v) >= 10 ? v.toFixed(1) : v.toFixed(2));

// TEMPORARY GUARD: distortion (catalog id 2) stays disabled in the FX
// picker — its C++ TU has not landed yet and the engine rejects the id.
// TODO: remove every use of FX_ID_DISTORTION when the distortion TU lands.
const FX_ID_DISTORTION = 2;

/** Snap a knob's raw drag output onto the catalog param's step/integer
 *  grid and clamp to [min, max] — the worklet treats clamping as the
 *  UI's job (no stepped knob control exists in the repo yet, so a plain
 *  numeric snap at the call site is the convention). */
function snapFxParam(p: FxParamDef, v: number): number {
  let out = v;
  if (p.integer) out = Math.round(out);
  else if (p.step > 0) out = Math.round(out / p.step) * p.step;
  return Math.max(p.min, Math.min(p.max, out));
}

export function ChannelDetailPanel(props: { channelIndex: number; slot?: { bus: number; slot: number } }) {
  const idx = props.channelIndex;
  const ch = () => channels[idx];

  // Section collapse state (default some to collapsed)
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({
    INPUT: true, EQ: true, FX: true,
  });
  const toggle = (s: string) => setCollapsed(c => ({ ...c, [s]: !c[s] }));
  const isCollapsed = (s: string) => !!collapsed()[s];

  // Compute panel width based on which sections are open.
  // Each section has a known min content width from its knobs/controls.
  const panelWidth = () => {
    let w = 100; // minimum
    if (!isCollapsed("INPUT")) w = Math.max(w, 360); // 6 comp knobs + labels + GR meter
    if (!isCollapsed("EQ")) w = Math.max(w, 230);
    if (!isCollapsed("FX")) w = Math.max(w, 300); // fx chain rows (reorder + toggle + picker)
    return w;
  };

  // ── FX chain (pair-owned rack) ─────────────────────────────────────
  // Even input strips own the 2t/2t+1 pair's chain (store fxChains
  // [pair][slot], helpers take the EVEN ch = idx); odd strips show a
  // pointer to the owner instead, bus slots (>= 128) get no chain at all.
  const fxPair = idx < NUM_CHANNELS && idx % 2 === 0 ? idx / 2 : null;

  // UI-local: which chain slot the edit area shows. Follows the first
  // enabled slot until the user picks one explicitly (OctOBX semantics).
  const [fxSlotSel, setFxSlotSel] = createSignal<number | null>(null);
  const selectedFxSlot = () => {
    if (fxSlotSel() !== null) return fxSlotSel()!;
    for (let s = 0; s < FX_SLOTS; s++) {
      const st = fxChains[fxPair!][s];
      if (st.fx >= 0 && st.enabled) return s;
    }
    return 0;
  };

  // Dropdown category order — derived from the catalog (first appearance
  // in id order), never hardcoded.
  const fxCategories = () => {
    const cats: string[] = [];
    for (const fx of fxCatalog()?.effects ?? []) if (!cats.includes(fx.category)) cats.push(fx.category);
    return cats;
  };

  // Fader / pan / mute / solo have no dedicated store helpers, so they reuse
  // the same inline setChannels + sendToWorklet pattern as ChannelStrip.
  const onFader = (pos: number) => {
    const gain = faderToGain(pos);
    setChannels(idx, "gain", gain);
    sendToWorklet({ type: "set-gain", ch: idx, gain });
  };
  const onPan = (val: number) => {
    setChannels(idx, "pan", val);
    sendToWorklet({ type: "set-pan", ch: idx, pan: val });
  };
  const onMute = () => {
    const muted = !ch().muted;
    setChannels(idx, "muted", muted);
    sendToWorklet({ type: "set-mute", ch: idx, muted });
  };
  const onSolo = () => {
    const soloed = !ch().soloed;
    setChannels(idx, "soloed", soloed);
    sendToWorklet({ type: "set-solo", ch: idx, soloed });
  };

  const fmtPan = (p: number) => {
    if (Math.abs(p) < 0.02) return "C";
    return p < 0 ? `L${Math.round(-p * 100)}` : `R${Math.round(p * 100)}`;
  };

  return (
    <div class="detail-panel" style={{ width: `${panelWidth()}px` }}>
      {/* ── Header ─────────────────────────────────────── */}
      <div class="detail-header">
        <input
          class="detail-name-input"
          type="text"
          value={props.slot && ch().name.startsWith("ch") ? `B${props.slot.bus + 1}-S${props.slot.slot + 1}` : ch().name}
          onInput={(e) => setChannelName(idx, e.currentTarget.value)}
          title="Channel name"
        />
      </div>

      {props.slot && (
        <div class="slot-source-row">
          <select
            class="slot-source-select"
            value={busChannels[props.slot.bus].sources[props.slot.slot] ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              if (v === "") clearBusSource(props.slot!.bus, props.slot!.slot);
              else setBusSource(props.slot!.bus, props.slot!.slot, parseInt(v));
            }}
            title="Input source for this slot"
          >
            <option value="">—</option>
            {Array.from({ length: NUM_CHANNELS }, (_, ch) => (
              <option value={ch}>{channels[ch]?.name || `ch${ch + 1}`}</option>
            ))}
          </select>
        </div>
      )}

      {/* ── INPUT / DYNAMICS (collapsible) ─────────────── */}
      <div class="detail-section">
        <div class="detail-section-divider collapsible" onClick={() => toggle("INPUT")}>
          <span class="detail-section-label">INPUT / DYN</span>
        </div>
        {!isCollapsed("INPUT") && (
        <>
        <div class="detail-input-row">
          <Knob
            label="TRIM"
            value={ch().inputGainDb}
            min={-24} max={24} defaultValue={0}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setInputGain(idx, v)}
          />
          <button
            class={`phase-btn ${ch().phaseInverted ? "active" : ""}`}
            onClick={() => setPhase(idx, !ch().phaseInverted)}
            title="Phase / polarity invert"
          >Ø</button>
          {!props.slot && (
            <button
              class={`btn-sm btn-main ${ch().mainAssigned ? "active" : ""}`}
              onClick={() => setMainAssign(idx, !ch().mainAssigned)}
              title="Main bus assign — off = this strip reaches master only through its bus slots"
            >MAIN</button>
          )}
          <label class="detail-select-label">LAW
            <select
              class="detail-select"
              value={ch().panLaw}
              onInput={(e) => setPanLaw(idx, parseInt(e.currentTarget.value))}
              title="Pan law"
            >
              {PAN_LAWS.map((label, i) => <option value={i}>{label}</option>)}
            </select>
          </label>
        </div>

      {/* ── GATE ───────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header">
          <span class="detail-section-label">GATE</span>
          <button
            class={`detail-toggle ${ch().gateEnabled ? "active" : "bypassed"}`}
            onClick={() => setGateEnabled(idx, !ch().gateEnabled)}
            title="Gate in / bypass"
          >{ch().gateEnabled ? "IN" : "BYP"}</button>
        </div>
        <div class="knob-row">
          <Knob label="THR" value={ch().gateThresholdDb} min={-80} max={0} defaultValue={-50}
            unit="dB" format={fmtDb} size={36} log
            onChange={(v) => setGateThreshold(idx, v)} />
          <Knob label="HYS" value={ch().gateHysteresisDb} min={0} max={24} defaultValue={6}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setGateHysteresis(idx, v)} />
          <Knob label="ATTK" value={ch().gateAttackMs} min={0.1} max={100} defaultValue={2}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setGateAttack(idx, v)} />
          <Knob label="REL" value={ch().gateReleaseMs} min={5} max={1000} defaultValue={100}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setGateRelease(idx, v)} />
          <Knob label="HOLD" value={ch().gateHoldMs} min={0} max={500} defaultValue={10}
            unit="ms" format={fmtMs} size={36}
            onChange={(v) => setGateHold(idx, v)} />
        </div>
      </div>

      {/* ── COMP ───────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header">
          <span class="detail-section-label">COMP</span>
          <button
            class={`detail-toggle ${ch().compEnabled ? "active" : "bypassed"}`}
            onClick={() => setCompEnabled(idx, !ch().compEnabled)}
            title="Compressor in / bypass"
          >{ch().compEnabled ? "IN" : "BYP"}</button>
        </div>
        <div class="knob-row">
          <Knob label="THR" value={ch().compThresholdDb} min={-60} max={0} defaultValue={-12}
            unit="dB" format={fmtDb} size={36} log
            onChange={(v) => setCompThreshold(idx, v)} />
          <Knob label="RATIO" value={ch().compRatio} min={1} max={20} defaultValue={3}
            format={fmtRatio} size={36}
            onChange={(v) => setCompRatio(idx, v)} />
          <Knob label="ATTK" value={ch().compAttackMs} min={0.1} max={100} defaultValue={5}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setCompAttack(idx, v)} />
          <Knob label="REL" value={ch().compReleaseMs} min={5} max={1000} defaultValue={100}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setCompRelease(idx, v)} />
          <Knob label="KNEE" value={ch().compKneeDb} min={0} max={12} defaultValue={3}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setCompKnee(idx, v)} />
          <Knob label="MKUP" value={ch().compMakeupDb} min={0} max={24} defaultValue={3}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setCompMakeup(idx, v)} />
        </div>
        <GrMeter reduction={-Math.abs(ch().compGrDb)} maxReduction={-20} label="GR" width={240} height={14} />
      </div>

      {/* ── EXPAND ─────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header">
          <span class="detail-section-label">EXPAND</span>
          <button
            class={`detail-toggle ${ch().expanderEnabled ? "active" : "bypassed"}`}
            onClick={() => setExpanderEnabled(idx, !ch().expanderEnabled)}
            title="Expander in / bypass"
          >{ch().expanderEnabled ? "IN" : "BYP"}</button>
        </div>
        <div class="knob-row">
          <Knob label="THR" value={ch().expanderThresholdDb} min={-80} max={0} defaultValue={-40}
            unit="dB" format={fmtDb} size={36} log
            onChange={(v) => setExpanderThreshold(idx, v)} />
          <Knob label="RATIO" value={ch().expanderRatio} min={1} max={10} defaultValue={2}
            format={fmtRatio} size={36}
            onChange={(v) => setExpanderRatio(idx, v)} />
          <Knob label="ATTK" value={ch().expanderAttackMs} min={0.1} max={100} defaultValue={5}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setExpanderAttack(idx, v)} />
          <Knob label="REL" value={ch().expanderReleaseMs} min={5} max={1000} defaultValue={100}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setExpanderRelease(idx, v)} />
        </div>
      </div>
        </>
        )}
      </div>

      {/* ── EQ (6-band) ────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header collapsible" onClick={() => toggle("EQ")}>
          <span class="detail-section-label">EQ</span>
          {!isCollapsed("EQ") && (
          <button
            class={`detail-toggle ${ch().eqBypassed ? "bypassed" : "active"}`}
            onClick={(e) => { e.stopPropagation(); setEqBypass(idx, !ch().eqBypassed); }}
            title="EQ in / bypass"
          >{ch().eqBypassed ? "BYP" : "IN"}</button>
          )}
        </div>
        {!isCollapsed("EQ") && (
        <>
        <div class="detail-eq-curve">
          <EQCurve channelIndex={idx} />
        </div>
        <div class="eq-bands">
          {EQ_BANDS.map((b, i) => (
            <div class="eq-band-row">
              <span class="eq-band-name">{b.name}</span>
              <div class="eq-band-knobs">
                {b.hasGain && (
                  <Knob label="G" value={ch().eqBands[i].gainDb} min={-12} max={12} defaultValue={0}
                    unit="dB" format={fmtDb} size={28}
                    onChange={(v) => setEqGain(idx, i, v)} />
                )}
                <Knob label="F" value={ch().eqBands[i].freqHz} min={20} max={20000} defaultValue={b.freq}
                  unit="Hz" format={fmtHz} size={28} log
                  onChange={(v) => setEqFreq(idx, i, v)} />
                <Knob label="Q" value={ch().eqBands[i].q} min={0.1} max={10} defaultValue={b.q}
                  format={(v) => v.toFixed(2)} size={28}
                  onChange={(v) => setEqQ(idx, i, v)} />
              </div>
            </div>
          ))}
        </div>
        </>
        )}
      </div>

      {/* ── FX CHAIN (pair-owned; even input strips only) ── */}
      {idx < NUM_CHANNELS && idx % 2 === 1 && (
        <div
          class="fx-pair-hint"
          title={`FX chains are pair-owned: strips 2t/2t+1 form one L/R pair and the pair's chain is owned and edited on the L strip (channel ${idx - 1}, "${channels[idx - 1].name}").`}
        >FX CHAIN → {channels[idx - 1].name}</div>
      )}
      {fxPair !== null && (
      <div class="detail-section">
        <div class="detail-section-divider collapsible" onClick={() => toggle("FX")} title="FX chain of this stereo pair — click to expand/collapse">
          <span class="detail-section-label">FX CHAIN</span>
        </div>
        {/* LED chain summary (control-rate — DOM, not canvas): hollow =
            empty, dim category color = loaded but BYP, lit = IN. */}
        <div class="detail-fx-leds">
          {fxChains[fxPair].map((st, s) => {
            const fx = st.fx >= 0 ? fxCatalog()?.effects[st.fx] : undefined;
            return (
              <span
                class={`fx-led${st.fx >= 0 ? ` loaded${fx ? ` cat-${fx.category}` : ""}${st.enabled ? " on" : ""}` : ""}`}
                title={fx ? `${s + 1}. ${fx.label} — ${st.enabled ? "IN" : "BYP"}` : `${s + 1}. (empty)`}
              />
            );
          })}
        </div>
        {!isCollapsed("FX") && (
          fxCatalog() === null ? (
            <div class="fx-unavailable" title="The engine FX catalog has not loaded — the wasm may not be ready, or the build predates the FX API.">
              FX unavailable — catalog not loaded
            </div>
          ) : (
          <>
            <div class="fx-chain-list">
              {fxChains[fxPair].map((st, s) => (
                <div
                  class={`fx-row${s === selectedFxSlot() ? " selected" : ""}`}
                  onClick={() => setFxSlotSel(s)}
                  title={`Chain slot ${s + 1} — click to edit its parameters below`}
                >
                  <button
                    class="fx-row-btn"
                    disabled={s === 0}
                    onClick={(e) => { e.stopPropagation(); moveFxSlot(idx, s, s - 1); }}
                    title="Move earlier in chain"
                  >▲</button>
                  <button
                    class="fx-row-btn"
                    disabled={s === FX_SLOTS - 1}
                    onClick={(e) => { e.stopPropagation(); moveFxSlot(idx, s, s + 1); }}
                    title="Move later in chain"
                  >▼</button>
                  <button
                    class={`detail-toggle ${st.enabled ? "active" : "bypassed"}`}
                    disabled={st.fx < 0}
                    onClick={(e) => { e.stopPropagation(); setFxEnabled(idx, s, !st.enabled); }}
                    title={st.fx >= 0 ? `${fxCatalog()?.effects[st.fx]?.label ?? "?"} in / bypass` : "Empty slot"}
                  >{st.enabled && st.fx >= 0 ? "IN" : "BYP"}</button>
                  <select
                    class="fx-slot-select"
                    value={st.fx}
                    onInput={(e) => setFxSlot(idx, s, parseInt(e.currentTarget.value))}
                    title={`Effect in chain slot ${s + 1}`}
                  >
                    <option value={-1}>(empty)</option>
                    {fxCategories().map((cat) => (
                      <optgroup label={cat.charAt(0).toUpperCase() + cat.slice(1)}>
                        {fxCatalog()!.effects.filter((f) => f.category === cat).map((f) => (
                          // TEMPORARY GUARD (see FX_ID_DISTORTION above):
                          // distortion stays visible-but-disabled until its
                          // C++ TU lands.
                          <option value={f.id} disabled={f.id === FX_ID_DISTORTION}>
                            {f.label}{f.id === FX_ID_DISTORTION ? " (unavailable)" : ""}
                          </option>
                        ))}
                      </optgroup>
                    ))}
                  </select>
                </div>
              ))}
            </div>
            <div class="fx-edit-area">
              {(() => {
                const s = selectedFxSlot();
                const st = fxChains[fxPair!][s];
                const fx = st.fx >= 0 ? fxCatalog()?.effects[st.fx] : undefined;
                if (!fx) return <div class="fx-edit-title">empty slot</div>;
                return (
                  <>
                    <div class="fx-edit-title">{fx.label} · {fx.stereo ? "stereo" : "mono"}</div>
                    <div class="knob-row">
                      {fx.params.map((p, pi) => (
                        <Knob
                          label={p.name}
                          value={st.params[pi] ?? p.default}
                          min={p.min} max={p.max} defaultValue={p.default}
                          format={fmtFx} size={28}
                          onChange={(v) => setFxParam(idx, s, pi, snapFxParam(p, v))}
                        />
                      ))}
                    </div>
                  </>
                );
              })()}
            </div>
          </>
          )
        )}
      </div>
      )}

      {/* ── PAN ───────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-divider">
          <span class="detail-section-label">PAN</span>
        </div>
        <div class="detail-pan-row">
          <input
            type="range"
            class="pan-slider"
            min={-1} max={1} step={0.01}
            value={ch().pan}
            onInput={(e) => onPan(parseFloat(e.currentTarget.value))}
            title="Pan"
          />
        </div>
      </div>

      {/* ── METER + FADER ──────────────────────────────── */}
      <div class="detail-section detail-output">
        <div class="detail-meter-fader">
          <MeterCanvas peakDb={ch().peakDb} rmsDb={ch().rmsDb} width={14} height={160} />
          <div class="fader-col">
            <span
              class={`strip-gated ${ch().degraded ? "on" : ""}`}
              title="Stream health gate: this strip's delivery is pathologically lossy/bursty (e.g. TSBPD latency below the network's jitter, constant SRT drops) — the mixer faded it to silence to protect the output. Raise the receive latency or fix the connection; the strip returns automatically once delivery is clean."
            >GATED</span>
            <span class="fader-val">{formatGainDb(ch().gain)}</span>
            <div class="fader-wrap">
              <input
                type="range"
                class="fader"
                min={0} max={1} step={0.001}
                value={gainToFader(ch().gain)}
                onInput={(e) => onFader(parseFloat(e.currentTarget.value))}
                onWheel={(e) => {
                  e.preventDefault();
                  const step = e.shiftKey ? 0.002 : 0.01;
                  const delta = e.deltaY > 0 ? -step : step;
                  const cur = gainToFader(ch().gain);
                  onFader(Math.max(0, Math.min(1, cur + delta)));
                }}
                title="Channel fader"
              />
            </div>
          </div>
        </div>
        <div class="detail-controls">
          <button
            class={`btn-sm btn-solo ${ch().soloed ? "active" : ""}`}
            onClick={onSolo}
            title="Solo"
          >S</button>
          <button
            class={`btn-sm btn-mute ${ch().muted ? "active" : ""}`}
            onClick={onMute}
            title="Mute"
          >M</button>
        </div>
      </div>
    </div>
  );
}
