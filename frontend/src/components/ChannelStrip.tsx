import { createSignal } from "solid-js";
import {
  channels, setChannels, sendToWorklet,
  faderToGain, gainToFader, formatGainDb,
} from "../stores/mixer";
import { MeterCanvas } from "./MeterCanvas";
import { EQCurve } from "./EQCurve";

const EQ_BANDS = [
  { name: "LOW", band: 1, freq: "120" },
  { name: "L-MID", band: 2, freq: "400" },
  { name: "MID", band: 3, freq: "1.5k" },
  { name: "H-MID", band: 4, freq: "5k" },
];

export function ChannelStrip(props: { index: number }) {
  const ch = () => channels[props.index];
  const [expanded, setExpanded] = createSignal(false);

  const onGain = (pos: number) => {
    const gain = faderToGain(pos);
    setChannels(props.index, "gain", gain);
    sendToWorklet({ type: "set-gain", ch: props.index, gain });
  };

  const onPan = (val: number) => {
    setChannels(props.index, "pan", val);
    sendToWorklet({ type: "set-pan", ch: props.index, pan: val });
  };

  const onEqGain = (band: number, db: number) => {
    setChannels(props.index, "eqBands", band - 1, "gainDb", db);
    sendToWorklet({ type: "set-eq-gain", ch: props.index, band, gainDb: db });
  };

  const onMute = () => {
    const muted = !ch().muted;
    setChannels(props.index, "muted", muted);
    sendToWorklet({ type: "set-mute", ch: props.index, muted });
  };

  const onSolo = () => {
    const soloed = !ch().soloed;
    setChannels(props.index, "soloed", soloed);
    sendToWorklet({ type: "set-solo", ch: props.index, soloed });
  };

  const onEqBypass = () => {
    const bypassed = !ch().eqBypassed;
    setChannels(props.index, "eqBypassed", bypassed);
    sendToWorklet({ type: "set-eq-bypass", ch: props.index, bypassed });
  };

  const fmtPan = (p: number) => {
    if (Math.abs(p) < 0.02) return "C";
    return p < 0 ? `L${Math.round(-p * 100)}` : `R${Math.round(p * 100)}`;
  };

  return (
    <div class={`channel-strip ${ch().muted ? "muted" : ""} ${ch().soloed ? "soloed" : ""}`}>
      {/* Header */}
      <div class="ch-header" onClick={() => setExpanded(!expanded())}>
        <span class="ch-num">{String(props.index + 1).padStart(2, "0")}</span>
        <span class="ch-expand">{expanded() ? "▾" : "▸"}</span>
      </div>

      {/* EQ Curve visualization */}
      <div class="eq-curve-container">
        <EQCurve channelIndex={props.index} />
      </div>

      {/* EQ Sliders */}
      <div class="eq-section">
        <div class="eq-header">
          <span class="section-label">EQ</span>
          <button
            class={`eq-bypass-btn ${ch().eqBypassed ? "bypassed" : "active"}`}
            onClick={onEqBypass}
          >{ch().eqBypassed ? "BYP" : "IN"}</button>
        </div>
        <div class="eq-sliders">
          {EQ_BANDS.map((b) => (
            <div class="eq-band">
              <span class="eq-val">
                {ch().eqBands[b.band - 1].gainDb > 0 ? "+" : ""}{ch().eqBands[b.band - 1].gainDb.toFixed(0)}
              </span>
              <div class="eq-slider-wrap">
                <input
                  type="range"
                  class="eq-slider"
                  min="-12" max="12" step="0.5"
                  value={ch().eqBands[b.band - 1].gainDb}
                  onInput={(e) => onEqGain(b.band, parseFloat(e.currentTarget.value))}
                  style={{ "writing-mode": "vertical-lr", direction: "rtl" }}
                />
                <div class="eq-slider-center-line" />
              </div>
              <span class="eq-band-label">{b.name}</span>
              <span class="eq-band-freq">{b.freq}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Pan + Meter + Fader row */}
      <div class="controls-row">
        {/* Pan */}
        <div class="pan-section">
          <input
            type="range"
            class="pan-slider"
            min="-1" max="1" step="0.01"
            value={ch().pan}
            onInput={(e) => onPan(parseFloat(e.currentTarget.value))}
          />
          <span class="pan-val">{fmtPan(ch().pan)}</span>
        </div>

        {/* Meter + Fader side by side */}
        <div class="meter-fader-row">
          <MeterCanvas width={8} height={140} />
          <div class="fader-wrap">
            <input
              type="range"
              class="fader"
              min="0" max="1" step="0.001"
              value={gainToFader(ch().gain)}
              onInput={(e) => onGain(parseFloat(e.currentTarget.value))}
              style={{ "writing-mode": "vertical-lr", direction: "rtl" }}
            />
          </div>
        </div>

        {/* Fader dB value */}
        <span class="fader-val">{formatGainDb(ch().gain)}</span>
      </div>

      {/* Mute / Solo */}
      <div class="ch-buttons">
        <button
          class={`btn-sm btn-solo ${ch().soloed ? "active" : ""}`}
          onClick={onSolo}
        >S</button>
        <button
          class={`btn-sm btn-mute ${ch().muted ? "active" : ""}`}
          onClick={onMute}
        >M</button>
      </div>
    </div>
  );
}
