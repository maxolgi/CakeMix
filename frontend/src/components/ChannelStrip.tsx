import { createSignal, Show } from "solid-js";
import {
  channels, setChannels, sendToWorklet,
  faderToGain, gainToFader, formatGainDb,
} from "../stores/mixer";
import { MeterCanvas } from "./MeterCanvas";

const EQ_BANDS = [
  { name: "Low", band: 1, freq: 120 },
  { name: "Lo-Mid", band: 2, freq: 400 },
  { name: "Mid", band: 3, freq: 1500 },
  { name: "Hi-Mid", band: 4, freq: 5000 },
];

export function ChannelStrip(props: { index: number }) {
  const ch = () => channels[props.index];

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
    const eqIdx = band - 1;
    setChannels(props.index, "eqBands", eqIdx, "gainDb", db);
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
    <div class="channel-strip">
      <div class="ch-header">
        <span class="ch-num">{props.index + 1}</span>
      </div>

      {/* EQ Section */}
      <div class="eq-section">
        <div class="section-label">EQ</div>
        <label class="eq-bypass">
          <input type="checkbox" checked={!ch().eqBypassed} onChange={onEqBypass} />
          <span>IN</span>
        </label>
        <div class="eq-sliders">
          {EQ_BANDS.map((b) => (
            <div class="eq-band">
              <span class="eq-val">{ch().eqBands[b.band - 1].gainDb > 0 ? "+" : ""}{ch().eqBands[b.band - 1].gainDb}</span>
              <input
                type="range"
                class="eq-slider"
                min="-12" max="12" step="0.5"
                value={ch().eqBands[b.band - 1].gainDb}
                onInput={(e) => onEqGain(b.band, parseFloat(e.currentTarget.value))}
                style={{ "writing-mode": "vertical-lr", direction: "rtl" }}
              />
              <span class="eq-band-label">{b.name}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Pan */}
      <div class="pan-section">
        <label>PAN</label>
        <input
          type="range"
          class="h-slider pan-slider"
          min="-1" max="1" step="0.01"
          value={ch().pan}
          onInput={(e) => onPan(parseFloat(e.currentTarget.value))}
        />
        <span class="pan-val">{fmtPan(ch().pan)}</span>
      </div>

      {/* Meter */}
      <div class="ch-meter-col">
        <MeterCanvas width={10} height={130} />
      </div>

      {/* Fader */}
      <div class="fader-section">
        <span class="fader-val">{formatGainDb(ch().gain)}</span>
        <input
          type="range"
          class="fader"
          min="0" max="1" step="0.001"
          value={gainToFader(ch().gain)}
          onInput={(e) => onGain(parseFloat(e.currentTarget.value))}
          style={{ "writing-mode": "vertical-lr", direction: "rtl" }}
        />
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
