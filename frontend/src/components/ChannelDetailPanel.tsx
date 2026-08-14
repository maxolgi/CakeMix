import { createSignal } from "solid-js";
import {
  channels,
  setInputGain, setPhase, setPanLaw, setChannelName,
  setEqGain, setEqFreq, setEqQ, setEqBypass,
  setCompEnabled, setCompThreshold, setCompRatio, setCompAttack, setCompRelease, setCompKnee, setCompMakeup,
  setGateEnabled, setGateThreshold, setGateHysteresis, setGateAttack, setGateRelease, setGateHold,
  setExpanderEnabled, setExpanderThreshold, setExpanderRatio, setExpanderAttack, setExpanderRelease,
  setAuxSend,
  routeToBus, routeToMaster,
  busChannels, setBusSource, clearBusSource, NUM_CHANNELS,
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

export function ChannelDetailPanel(props: { channelIndex: number; slot?: { bus: number; slot: number } }) {
  const idx = props.channelIndex;
  const ch = () => channels[idx];

  // Section collapse state (default some to collapsed)
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({
    INPUT: true, EQ: true, SENDS: true,
  });
  const toggle = (s: string) => setCollapsed(c => ({ ...c, [s]: !c[s] }));
  const isCollapsed = (s: string) => !!collapsed()[s];

  // Compute panel width based on which sections are open.
  // Each section has a known min content width from its knobs/controls.
  const panelWidth = () => {
    let w = 100; // minimum
    if (!isCollapsed("INPUT")) w = Math.max(w, 360); // 6 comp knobs + labels + GR meter
    if (!isCollapsed("EQ")) w = Math.max(w, 230);
    if (!isCollapsed("SENDS")) w = Math.max(w, 250);
    return w;
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
        <GrMeter reduction={0} maxReduction={-20} label="GR" width={240} height={14} />
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
        <GrMeter reduction={0} maxReduction={-20} label="GR" width={240} height={14} />
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

      {/* ── SENDS (4 aux) ──────────────────────────────── */}
      {!props.slot && (
      <div class="detail-section">
        <div class="detail-section-divider collapsible" onClick={() => toggle("SENDS")}>
          <span class="detail-section-label">SENDS</span>
        </div>
        {!isCollapsed("SENDS") && (
        <>
        {[0, 1, 2, 3].map((si) => {
          const s = () => ch().sends[si];
          const level = () => {
            const l = s().levelDb;
            return isFinite(l) ? l : -60;
          };
          const hasBus = () => s().busId !== null;
          return (
            <div class={`send-row ${hasBus() ? "" : "disabled"}`}>
              <span class="send-num">S{si + 1}</span>
              <Knob
                label="LVL"
                value={level()}
                min={-60} max={6} defaultValue={-60}
                unit="dB" format={fmtDb} size={36} log
                onChange={(v) => { if (hasBus()) setAuxSend(idx, si, v, s().preFader, s().busId); }}
              />
              <button
                class={`send-pre-btn ${s().preFader ? "active" : ""}`}
                onClick={() => setAuxSend(idx, si, level(), !s().preFader, s().busId)}
                disabled={!hasBus()}
                title={s().preFader ? "Pre-fader send (click for post)" : "Post-fader send (click for pre)"}
              >{s().preFader ? "PRE" : "PST"}</button>
              <select
                class="send-bus-select"
                value={s().busId ?? ""}
                onInput={(e) => {
                  const v = e.currentTarget.value;
                  const busId = v ? parseInt(v) : null;
                  setAuxSend(idx, si, level(), s().preFader, busId);
                }}
                title={hasBus() ? "Send target bus" : "Pick a bus to enable this send"}
              >
                <option value="">—</option>
                {busChannels.map((bus, id) => (
                  <option value={id}>{bus.name}</option>
                ))}
              </select>
            </div>
          );
        })}
        </>
        )}
      </div>
      )}

      {/* ── PAN + ROUTING ──────────────────────────────── */}
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
        <div class="detail-routing-row">
          <div class="detail-select-label">ROUTE</div>
          <select
            class="detail-select detail-routing-select"
            value={ch().outputBus === "master" ? "master" : `bus-${ch().outputBus}`}
            onInput={(e) => {
              const val = e.currentTarget.value;
              if (val === "master") routeToMaster(idx);
              else routeToBus(idx, parseInt(val.replace("bus-", "")));
            }}
            title="Channel output routing — master or bus"
          >
            <option value="master">MASTER</option>
            {busChannels.map((bus, id) => (
              <option value={`bus-${id}`}>{bus.name}</option>
            ))}
          </select>
        </div>
      </div>

      {/* ── METER + FADER ──────────────────────────────── */}
      <div class="detail-section detail-output">
        <div class="detail-meter-fader">
          <MeterCanvas peakDb={ch().peakDb} rmsDb={ch().rmsDb} width={14} height={160} />
          <div class="fader-col">
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
