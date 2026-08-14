import { createSignal } from "solid-js";
import {
  busChannels,
  NUM_CHANNELS,
  setBusSource, clearBusSource,
  setBusGain, setBusMute,
  setBusEqGain, setBusEqFreq, setBusEqQ, setBusEqBypass,
  setBusCompEnabled, setBusCompThreshold, setBusCompRatio, setBusCompAttack, setBusCompRelease, setBusCompKnee, setBusCompMakeup,
  setBusGateEnabled, setBusGateThreshold, setBusGateHysteresis, setBusGateAttack, setBusGateRelease, setBusGateHold,
  setBusExpanderEnabled, setBusExpanderThreshold, setBusExpanderRatio, setBusExpanderAttack, setBusExpanderRelease,
  faderToGain, gainToFader, formatGainDb,
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

const fmtDb = (v: number) => v.toFixed(1);
const fmtRatio = (v: number) => (v >= 20 ? "20:1" : v.toFixed(1) + ":1");
const fmtHz = (v: number) => (v >= 1000 ? (v / 1000).toFixed(1) + "k" : v.toFixed(0));
const fmtMs = (v: number) => (v < 1 ? v.toFixed(2) : v < 10 ? v.toFixed(1) : v.toFixed(0));

export function BusStrip(props: { busIndex: number }) {
  const bus = () => busChannels[props.busIndex];

  // Section collapse state (default some to collapsed) — same mechanism as channels.
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({
    INPUT: true, EQ: true,
  });
  const toggle = (s: string) => setCollapsed(c => ({ ...c, [s]: !c[s] }));
  const isCollapsed = (s: string) => !!collapsed()[s];

  // Compute panel width based on which sections are open.
  // Each section has a known min content width from its knobs/controls.
  const panelWidth = () => {
    let w = 100; // minimum
    if (!isCollapsed("INPUT")) w = Math.max(w, 360); // 6 comp knobs + labels + GR meter
    if (!isCollapsed("EQ")) w = Math.max(w, 230);
    return w;
  };

  const onFader = (pos: number) => setBusGain(props.busIndex, faderToGain(pos));
  const onMute = () => setBusMute(props.busIndex, !bus().muted);

  return (
    <div class="detail-panel bus-detail" style={{ width: `${panelWidth()}px` }}>
      {/* ── Header ─────────────────────────────────────── */}
      <div class="detail-header">
        <input
          class="detail-name-input"
          type="text"
          value={bus().name}
          readonly
          title={`Bus ${props.busIndex + 1}`}
        />
      </div>

      {/* ── INPUT / DYN (collapsible) ──────────────────── */}
      <div class="detail-section">
        <div class="detail-section-divider collapsible" onClick={() => toggle("INPUT")}>
          <span class="detail-section-label">SOURCES</span>
        </div>
        {!isCollapsed("INPUT") && (
        <>
        {Array.from({ length: 16 }, (_, slot) => (
          <div class="bus-source-row">
            <span class="bus-source-num">{slot + 1}</span>
            <select
              class="bus-source-select"
              value={bus().sources[slot] ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                if (v === "") clearBusSource(props.busIndex, slot);
                else setBusSource(props.busIndex, slot, parseInt(v));
              }}
              title={`Source slot ${slot + 1}`}
            >
              <option value="">—</option>
              {Array.from({ length: NUM_CHANNELS }, (_, ch) => (
                <option value={ch}>ch{ch + 1}</option>
              ))}
            </select>
          </div>
        ))}

      {/* ── GATE ───────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header">
          <span class="detail-section-label">GATE</span>
          <button
            class={`detail-toggle ${bus().gateEnabled ? "active" : "bypassed"}`}
            onClick={() => setBusGateEnabled(props.busIndex, !bus().gateEnabled)}
            title="Gate in / bypass"
          >{bus().gateEnabled ? "IN" : "BYP"}</button>
        </div>
        <div class="knob-row">
          <Knob label="THR" value={bus().gateThresholdDb} min={-80} max={0} defaultValue={-50}
            unit="dB" format={fmtDb} size={36} log
            onChange={(v) => setBusGateThreshold(props.busIndex, v)} />
          <Knob label="HYS" value={bus().gateHysteresisDb} min={0} max={24} defaultValue={6}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setBusGateHysteresis(props.busIndex, v)} />
          <Knob label="ATTK" value={bus().gateAttackMs} min={0.1} max={100} defaultValue={2}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setBusGateAttack(props.busIndex, v)} />
          <Knob label="REL" value={bus().gateReleaseMs} min={5} max={1000} defaultValue={100}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setBusGateRelease(props.busIndex, v)} />
          <Knob label="HOLD" value={bus().gateHoldMs} min={0} max={500} defaultValue={10}
            unit="ms" format={fmtMs} size={36}
            onChange={(v) => setBusGateHold(props.busIndex, v)} />
        </div>
        <GrMeter reduction={0} maxReduction={-20} label="GR" width={240} height={14} />
      </div>

      {/* ── COMP ───────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header">
          <span class="detail-section-label">COMP</span>
          <button
            class={`detail-toggle ${bus().compEnabled ? "active" : "bypassed"}`}
            onClick={() => setBusCompEnabled(props.busIndex, !bus().compEnabled)}
            title="Compressor in / bypass"
          >{bus().compEnabled ? "IN" : "BYP"}</button>
        </div>
        <div class="knob-row">
          <Knob label="THR" value={bus().compThresholdDb} min={-60} max={0} defaultValue={-12}
            unit="dB" format={fmtDb} size={36} log
            onChange={(v) => setBusCompThreshold(props.busIndex, v)} />
          <Knob label="RATIO" value={bus().compRatio} min={1} max={20} defaultValue={3}
            format={fmtRatio} size={36}
            onChange={(v) => setBusCompRatio(props.busIndex, v)} />
          <Knob label="ATTK" value={bus().compAttackMs} min={0.1} max={100} defaultValue={5}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setBusCompAttack(props.busIndex, v)} />
          <Knob label="REL" value={bus().compReleaseMs} min={5} max={1000} defaultValue={100}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setBusCompRelease(props.busIndex, v)} />
          <Knob label="KNEE" value={bus().compKneeDb} min={0} max={12} defaultValue={3}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setBusCompKnee(props.busIndex, v)} />
          <Knob label="MKUP" value={bus().compMakeupDb} min={0} max={24} defaultValue={3}
            unit="dB" format={fmtDb} size={36}
            onChange={(v) => setBusCompMakeup(props.busIndex, v)} />
        </div>
        <GrMeter reduction={0} maxReduction={-20} label="GR" width={240} height={14} />
      </div>

      {/* ── EXPAND ─────────────────────────────────────── */}
      <div class="detail-section">
        <div class="detail-section-header">
          <span class="detail-section-label">EXPAND</span>
          <button
            class={`detail-toggle ${bus().expanderEnabled ? "active" : "bypassed"}`}
            onClick={() => setBusExpanderEnabled(props.busIndex, !bus().expanderEnabled)}
            title="Expander in / bypass"
          >{bus().expanderEnabled ? "IN" : "BYP"}</button>
        </div>
        <div class="knob-row">
          <Knob label="THR" value={bus().expanderThresholdDb} min={-80} max={0} defaultValue={-40}
            unit="dB" format={fmtDb} size={36} log
            onChange={(v) => setBusExpanderThreshold(props.busIndex, v)} />
          <Knob label="RATIO" value={bus().expanderRatio} min={1} max={10} defaultValue={2}
            format={fmtRatio} size={36}
            onChange={(v) => setBusExpanderRatio(props.busIndex, v)} />
          <Knob label="ATTK" value={bus().expanderAttackMs} min={0.1} max={100} defaultValue={5}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setBusExpanderAttack(props.busIndex, v)} />
          <Knob label="REL" value={bus().expanderReleaseMs} min={5} max={1000} defaultValue={100}
            unit="ms" format={fmtMs} size={36} log
            onChange={(v) => setBusExpanderRelease(props.busIndex, v)} />
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
            class={`detail-toggle ${bus().eqBypassed ? "bypassed" : "active"}`}
            onClick={(e) => { e.stopPropagation(); setBusEqBypass(props.busIndex, !bus().eqBypassed); }}
            title="EQ in / bypass"
          >{bus().eqBypassed ? "BYP" : "IN"}</button>
          )}
        </div>
        {!isCollapsed("EQ") && (
        <>
        <div class="detail-eq-curve">
          <EQCurve channelIndex={props.busIndex} bus />
        </div>
        <div class="eq-bands">
          {EQ_BANDS.map((b, i) => (
            <div class="eq-band-row">
              <span class="eq-band-name">{b.name}</span>
              <div class="eq-band-knobs">
                {b.hasGain && (
                  <Knob label="G" value={bus().eqBands[i].gainDb} min={-12} max={12} defaultValue={0}
                    unit="dB" format={fmtDb} size={28}
                    onChange={(v) => setBusEqGain(props.busIndex, i, v)} />
                )}
                <Knob label="F" value={bus().eqBands[i].freqHz} min={20} max={20000} defaultValue={b.freq}
                  unit="Hz" format={fmtHz} size={28} log
                  onChange={(v) => setBusEqFreq(props.busIndex, i, v)} />
                <Knob label="Q" value={bus().eqBands[i].q} min={0.1} max={10} defaultValue={b.q}
                  format={(v) => v.toFixed(2)} size={28}
                  onChange={(v) => setBusEqQ(props.busIndex, i, v)} />
              </div>
            </div>
          ))}
        </div>
        </>
        )}
      </div>

      {/* ── METER + FADER ──────────────────────────────── */}
      <div class="detail-section detail-output">
        <div class="detail-meter-fader">
          <MeterCanvas peakDb={bus().peakDb} rmsDb={bus().rmsDb} width={14} height={160} />
          <div class="fader-col">
            <span class="fader-val">{formatGainDb(bus().gain)}</span>
            <div class="fader-wrap">
              <input
                type="range"
                class="fader"
                min={0} max={1} step={0.001}
                value={gainToFader(bus().gain)}
                onInput={(e) => onFader(parseFloat(e.currentTarget.value))}
                onWheel={(e) => {
                  e.preventDefault();
                  const step = e.shiftKey ? 0.002 : 0.01;
                  const delta = e.deltaY > 0 ? -step : step;
                  const cur = gainToFader(bus().gain);
                  onFader(Math.max(0, Math.min(1, cur + delta)));
                }}
                title="Bus fader"
              />
            </div>
          </div>
        </div>
        <div class="detail-controls">
          <button
            class={`btn-sm btn-mute ${bus().muted ? "active" : ""}`}
            onClick={onMute}
            title="Mute"
          >M</button>
        </div>
      </div>
    </div>
  );
}
