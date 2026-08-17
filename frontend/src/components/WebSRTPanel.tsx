import { createSignal, For, Show } from "solid-js";
import {
  websrtStatus,
  websrtStatusDetail,
  websrtPids,
  websrtLatencyMs,
  setWebsrtLatencyMs,
  websrtTarget,
  connectWebsrt,
  disconnectWebsrt,
} from "../websrt/store";
import {
  publishStatus,
  publishStatusDetail,
  publishStats,
  publishStreamName,
  publishTarget,
  publishChannels,
  setPublishChannels,
  publishSource,
  setPublishSource,
  publishBus,
  publishLatencyMs,
  setPublishLatencyMs,
  type PublishChannels,
  connectPublish,
  disconnectPublish,
} from "../websrt/publish";
import { wasmReady, isRunning, setIsRunning, sendToWorklet } from "../stores/mixer";

const LATENCY_OPTIONS = [120, 250, 500, 1000, 2000];
const CHANNEL_OPTIONS: { value: PublishChannels; label: string }[] = [
  { value: 2, label: "2 (master)" },
  { value: 16, label: "16" },
  { value: 32, label: "32" },
  { value: 64, label: "64" },
  { value: 128, label: "128" },
];

const mixerChRange = (chStart: number, channelCount: number) =>
  channelCount > 1 ? `${chStart + 1}–${chStart + channelCount}` : `${chStart + 1}`;

/** Settings drawer — engine controls, test tones, WebSRT receive + publish.
 *  Toggled from the top bar (App owns `expanded`). */
export function WebSRTPanel(props: { expanded: boolean }) {
  const [tonesOn, setTonesOn] = createSignal(false);

  const active = () => {
    const s = websrtStatus();
    return s === "connected" || s === "connecting";
  };

  const pubActive = () => {
    const s = publishStatus();
    return s === "connected" || s === "connecting";
  };

  const startEngine = () => {
    if (!isRunning()) {
      sendToWorklet({ type: "start" });
      setIsRunning(true);
    }
  };
  const stopEngine = () => {
    sendToWorklet({ type: "stop" });
    setIsRunning(false);
  };

  const toggleEngine = () => {
    if (isRunning()) stopEngine();
    else startEngine();
  };

  const toggleTones = () => {
    const on = !tonesOn();
    setTonesOn(on);
    sendToWorklet({ type: "tones", on });
  };

  return (
    <Show when={props.expanded}>
      <div class="settings-body">
        <div class="websrt-subsection">
          <div class="websrt-section-controls">
            <button
              class={`btn ${active() ? "btn-stop" : "btn-start"}`}
              disabled={websrtStatus() === "connecting"}
              onClick={() => (active() ? disconnectWebsrt() : connectWebsrt())}
              title={active()
                ? "Connected — click to disconnect the WebSRT receiver and stop the worker"
                : "Connect the WebSRT receiver to the target gateway (URL below)"}
            >RECEIVE</button>
            <label class="detail-select-label">LATENCY
              <select
                class="detail-select"
                value={String(websrtLatencyMs())}
                onInput={(e) => setWebsrtLatencyMs(parseInt(e.currentTarget.value, 10))}
                disabled={websrtStatus() === "connected"}
                title="TSBPD latency — higher = more robust, use ~1000 ms for LAN PCM. Disabled while connected: reconnect required to apply."
              >
                <For each={LATENCY_OPTIONS}>
                  {(ms) => <option value={String(ms)}>{ms} ms</option>}
                </For>
              </select>
            </label>
            <input
              class="websrt-input websrt-input-url"
              type="text"
              value={websrtTarget.url()}
              onInput={(e) => websrtTarget.setUrl(e.currentTarget.value)}
              disabled={active()}
              placeholder="this page — or https://192.168.1.214:5173/?stream=audio"
              title="Web-viewer URL of a WebSRT gateway. Host, stream name and token are parsed from it; cert hash + WT port are fetched from its /cert-hash.js. Empty = the gateway serving this page, stream default."
            />
            <button
              class={`btn btn-engine ${isRunning() ? "btn-stop" : "btn-start"}`}
              disabled={!wasmReady()}
              onClick={toggleEngine}
              title="Run / freeze the mixer engine. Starts automatically when WebSRT connects."
            >{isRunning() ? "ENGINE ON" : "ENGINE OFF"}</button>
            <button
              class={`btn ${tonesOn() ? "btn-stop" : "btn-start"}`}
              onClick={toggleTones}
              title="Sine tones (A major chord) into mixer inputs 1–4. Only audible while the engine runs."
            >{tonesOn() ? "TONES ON" : "TONES OFF"}</button>
            <span
              class={`websrt-pill ${websrtStatus()}`}
              title={`WebSRT connection status: ${websrtStatus()}`}
            >{websrtStatus() === "disconnected" ? "" : websrtStatus()}</span>
          </div>

          <Show when={websrtStatusDetail()}>
            <div class="websrt-status-detail" title={websrtStatusDetail()}>
              {websrtStatusDetail()}
            </div>
          </Show>

          <Show
            when={active() && websrtPids().length > 0}
          >
            <div class="websrt-pid-scroll">
              <table class="websrt-pid-table">
                <thead title="Audio PIDs are auto-discovered from the stream and mapped in arrival order to consecutive mixer input channels.">
                  <tr>
                    <th>PID</th>
                    <th>CH</th>
                    <th>MIXER CH</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={websrtPids()}>
                    {(p) => (
                      <tr>
                        <td>{p.pid}</td>
                        <td>{p.channelCount}</td>
                        <td>{mixerChRange(p.chStart, p.channelCount)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </div>

        <div class="websrt-subsection">
          <div class="websrt-section-controls">
            <button
              class={`btn ${pubActive() ? "btn-stop" : "btn-start"}`}
              disabled={publishStatus() === "connecting"}
              onClick={() => (pubActive() ? disconnectPublish() : connectPublish())}
              title={pubActive()
                ? "Publishing — click to stop: closes the output tap and the publish worker's SRT/WebTransport session"
                : `Publish the mixer output as SMPTE 302M PCM (48 kHz, no codecs) to the gateway, stream "${publishStreamName()}"`}
            >PUBLISH</button>
            <label class="detail-select-label">LATENCY
              <select
                class="detail-select"
                value={String(publishLatencyMs())}
                onInput={(e) => setPublishLatencyMs(parseInt(e.currentTarget.value, 10))}
                disabled={pubActive()}
                title="Publish TSBPD latency — how long the gateway buffers our outgoing stream before fan-out. Independent of the receive latency. Disabled while connected: reconnect required to apply."
              >
                <For each={LATENCY_OPTIONS}>
                  {(ms) => <option value={String(ms)}>{ms} ms</option>}
                </For>
              </select>
            </label>
            <label class="detail-select-label">CHANNELS
              <select
                class="detail-select"
                value={String(publishChannels())}
                onInput={(e) => setPublishChannels(parseInt(e.currentTarget.value, 10) as PublishChannels)}
                disabled={pubActive()}
                title="Output channel count — packed as ceil(N/2) stereo s302m PIDs (PID i = channels 2i/2i+1), discovered by receivers via the PMT. 2 = the master stereo mix. 16–128 = channel direct outs: mono per channel, tapped after input gain, gate, compressor, EQ and fader (pre-pan); muted channels publish silence. Changeable only while disconnected: the PID set is fixed at connect."
              >
                <For each={CHANNEL_OPTIONS}>
                  {(o) => <option value={String(o.value)}>{o.label}</option>}
                </For>
              </select>
            </label>
            <Show when={publishChannels() === 2}>
              <label class="detail-select-label">SOURCE
                <select
                  class="detail-select"
                  value={publishSource() === "bus" ? String(publishBus()) : "master"}
                  onInput={(e) => {
                    const v = e.currentTarget.value;
                    if (v === "master") setPublishSource("master");
                    else setPublishSource("bus", parseInt(v, 10));
                  }}
                  disabled={pubActive()}
                  title="Publish source (stereo only, so available just at channels = 2) — Master: the main stereo mix. Bus 1–8: that bus's stereo output, independent of its feeds-master setting (works for monitor/N-1 mixes). Changeable only while disconnected."
                >
                  <option value="master">Master</option>
                  <For each={Array.from({ length: 8 }, (_, i) => i)}>
                    {(b) => <option value={String(b)}>Bus {b + 1}</option>}
                  </For>
                </select>
              </label>
            </Show>
            <span
              class={`websrt-pill ${publishStatus()}`}
              title={`WebSRT publish status: ${publishStatus()}`}
            >{publishStatus() === "disconnected" ? "" : publishStatus()}</span>
          </div>

          <div
            class="websrt-status-detail"
            title={`Publish stream name and gateway target — same discovery as the receive path (?pubstream / ?host / ?port)`}
          >{publishTarget()
              ? `stream ${publishStreamName()}${publishSource() === "bus" ? ` · bus ${publishBus() + 1}` : ""} → ${publishTarget()}`
              : `stream ${publishStreamName()} → not connected`}</div>

          <Show when={publishStats()}>
            {(s) => (
              <div
                class="websrt-status-detail"
                title="Publish link stats (last second): TS payload bitrate · SRT round-trip time · cumulative tx-lost packets"
              >{`${s().kbps} kb/s · rtt ${s().rttMs.toFixed(0)} ms · tx loss ${s().txLoss}`}</div>
            )}
          </Show>

          <Show when={publishStatusDetail()}>
            <div class="websrt-status-detail" title={publishStatusDetail()}>
              {publishStatusDetail()}
            </div>
          </Show>
        </div>
      </div>
    </Show>
  );
}
