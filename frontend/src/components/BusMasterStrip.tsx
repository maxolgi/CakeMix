import {
  busChannels, setBusGain, setBusMute, setBusFeedsMain,
  faderToGain, gainToFader, formatGainDb,
} from "../stores/mixer";
import { MeterCanvas } from "./MeterCanvas";

export function BusMasterStrip(props: { bus: number }) {
  const bus = () => busChannels[props.bus];
  const onFader = (pos: number) => { setBusGain(props.bus, faderToGain(pos)); };

  return (
    <div class="detail-panel bus-master" style={{ width: "100px", "margin-left": "auto" }}>
      <div class="detail-header">
        <input class="detail-name-input" type="text" value={`BUS ${props.bus + 1}`} readonly title="Bus master" />
      </div>

      <div class="detail-section detail-output">
        <div class="detail-meter-fader">
          <MeterCanvas peakDb={bus().peakDb} rmsDb={bus().rmsDb} width={10} height={160} />
          <div class="fader-col">
            <span class="fader-val">{formatGainDb(bus().gain)}</span>
            <div class="fader-wrap">
              <input type="range" class="fader" min={0} max={1} step={0.001}
                value={gainToFader(bus().gain)}
                onInput={(e) => onFader(parseFloat(e.currentTarget.value))}
                onWheel={(e) => { e.preventDefault();
                  const step = e.shiftKey ? 0.002 : 0.01;
                  const delta = e.deltaY > 0 ? -step : step;
                  onFader(Math.max(0, Math.min(1, gainToFader(bus().gain) + delta)));
                }}
                title="Bus fader"
              />
            </div>
          </div>
        </div>
        <div class="detail-controls">
          <button
            class={`btn-sm feeds-main ${bus().feedsMain ? "active" : ""}`}
            onClick={() => setBusFeedsMain(props.bus, !bus().feedsMain)}
            title="Feeds master — off = independent bus mix (monitor/N-1), still publishable"
          >FEEDS MAIN</button>
          <button
            class={`btn-sm btn-mute ${bus().muted ? "active" : ""}`}
            onClick={() => setBusMute(props.bus, !bus().muted)}
            title="Bus mute"
          >M</button>
        </div>
      </div>
    </div>
  );
}
