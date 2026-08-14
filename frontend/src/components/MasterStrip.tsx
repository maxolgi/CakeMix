import { createSignal } from "solid-js";
import {
  meterData, masterGain, setMasterGain,
  limiterEnabled, setLimiterEnabled,
  limiterCeiling, setLimiterCeiling,
  limiterRelease, setLimiterRelease,
  faderToGain, gainToFader, formatGainDb,
} from "../stores/mixer";
import { Knob } from "./Knob";
import { GrMeter } from "./GrMeter";
import { MeterCanvas } from "./MeterCanvas";

const fmtDb = (v: number) => v.toFixed(1);
const fmtDbInf = (db: number) => db <= -60 ? "−∞" : db.toFixed(1);
const fmtMs = (v: number) => (v < 10 ? v.toFixed(1) : v.toFixed(0));

export function MasterStrip() {
  const [collapsed, setCollapsed] = createSignal<Record<string, boolean>>({});
  const toggle = (s: string) => setCollapsed(c => ({ ...c, [s]: !c[s] }));
  const isCollapsed = (s: string) => !!collapsed()[s];

  const onFader = (pos: number) => { setMasterGain(faderToGain(pos)); };

  return (
    <div class="detail-panel master-detail" style={{ width: "100px", "margin-left": "auto" }}>
      <div class="detail-header">
        <input class="detail-name-input" type="text" value="MASTER" readonly title="Master bus" />
      </div>

      <div class="detail-section">
        <div class="detail-section-header collapsible" onClick={() => toggle("INPUT")}>
          <span class="detail-section-label">LIMITER</span>
          <button
            class={`detail-toggle ${limiterEnabled() ? "active" : "bypassed"}`}
            onClick={(e) => { e.stopPropagation(); setLimiterEnabled(!limiterEnabled()); }}
            title="Limiter in / bypass"
          >{limiterEnabled() ? "IN" : "BYP"}</button>
        </div>
        {!isCollapsed("INPUT") && (
        <>
        <div class="knob-row">
          <Knob label="CEIL" value={limiterCeiling()} min={-12} max={0} defaultValue={-0.3}
            unit="dB" format={fmtDb} size={36} onChange={(v) => setLimiterCeiling(v)} />
          <Knob label="REL" value={limiterRelease()} min={5} max={500} defaultValue={50}
            unit="ms" format={fmtMs} size={36} log onChange={(v) => setLimiterRelease(v)} />
        </div>
        <GrMeter reduction={meterData().limiterGr} maxReduction={-20} label="GR" width={240} height={14} />
        </>
        )}
      </div>

      <div class="detail-section detail-output">
        <div class="master-meter-readout">
          <span>{fmtDbInf(meterData().peakL)}</span>
          <span class={`clip ${meterData().clip ? "active" : ""}`}>CLIP</span>
          <span>{fmtDbInf(meterData().peakR)}</span>
        </div>
        <div class="detail-meter-fader">
          <MeterCanvas peakDb={meterData().peakL} rmsDb={meterData().rmsL} width={10} height={160} />
          <div class="fader-col">
            <span class="fader-val">{formatGainDb(masterGain())}</span>
            <div class="fader-wrap">
              <input type="range" class="fader" min={0} max={1} step={0.001}
                value={gainToFader(masterGain())}
                onInput={(e) => onFader(parseFloat(e.currentTarget.value))}
                onWheel={(e) => { e.preventDefault();
                  const step = e.shiftKey ? 0.002 : 0.01;
                  const delta = e.deltaY > 0 ? -step : step;
                  onFader(Math.max(0, Math.min(1, gainToFader(masterGain()) + delta)));
                }}
                title="Master fader"
              />
            </div>
          </div>
          <MeterCanvas peakDb={meterData().peakR} rmsDb={meterData().rmsR} width={10} height={160} />
        </div>
      </div>
    </div>
  );
}
