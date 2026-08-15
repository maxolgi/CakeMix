import { createSignal, For, Show } from "solid-js";
import {
  websrtStatus,
  websrtStatusDetail,
  websrtPids,
  websrtLatencyMs,
  setWebsrtLatencyMs,
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

  const toggleTones = () => {
    const on = !tonesOn();
    setTonesOn(on);
    if (on) startEngine(); // tones are inaudible with the engine stopped
    sendToWorklet({ type: "tones", on });
  };

  return (
    <Show when={props.expanded}>
      <div class="settings-body">
        <div class="websrt-subsection">
          <div class="websrt-subsection-head">
            <span class="websrt-label">ENGINE</span>
            <span class={`websrt-pill ${isRunning() ? "connected" : "disconnected"}`}>
              {isRunning() ? "running" : "stopped"}
            </span>
          </div>
          <div class="websrt-controls">
            <button
              class="btn btn-start"
              disabled={!wasmReady() || isRunning()}
              onClick={startEngine}
              title="Run the mixer engine (starts automatically when WebSRT connects)"
            >START</button>
            <button
              class="btn btn-stop"
              disabled={!isRunning()}
              onClick={stopEngine}
              title="Freeze the mixer engine (meters fall to zero; publish keeps streaming silence)"
            >STOP</button>
          </div>
        </div>

        <div class="websrt-subsection">
          <div class="websrt-subsection-head">
            <span class="websrt-label">TEST TONES</span>
          </div>
          <div class="websrt-controls">
            <button
              class={`btn ${tonesOn() ? "btn-stop" : "btn-start"}`}
              onClick={toggleTones}
              title="Sine tones (A major chord) into mixer inputs 1–4. Enabling also starts the engine."
            >{tonesOn() ? "TONES ON" : "TONES OFF"}</button>
          </div>
        </div>

        <div class="websrt-subsection">
          <div class="websrt-subsection-head">
            <span class="websrt-label">RECEIVE</span>
            <span
              class={`websrt-pill ${websrtStatus()}`}
              title={`WebSRT connection status: ${websrtStatus()}`}
            >{websrtStatus()}</span>
          </div>

          <div class="websrt-controls">
            <button
              class={`btn ${active() ? "btn-stop" : "btn-start"}`}
              disabled={websrtStatus() === "connecting"}
              onClick={() => (active() ? disconnectWebsrt() : connectWebsrt())}
              title={active()
                ? "Disconnect the WebSRT receiver and stop the worker"
                : "Fetch same-origin /cert-hash.js and connect to the gateway's WebTransport endpoint"}
            >{active() ? "DISCONNECT" : "CONNECT"}</button>
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
          </div>

          <div class="websrt-status-detail" title={websrtStatusDetail()}>
            {websrtStatusDetail()}
          </div>

          <Show
            when={websrtPids().length > 0}
            fallback={<div class="websrt-pid-empty">No audio PIDs yet — waiting for PCM</div>}
          >
            <div class="websrt-pid-scroll">
              <table class="websrt-pid-table">
                <thead title="Audio PIDs are auto-discovered from the stream and mapped in arrival order to consecutive mixer input channels.">
                  <tr>
                    <th>PID</th>
                    <th>CH</th>
                    <th>MIXER CH</th>
                    <th>PCM</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={websrtPids()}>
                    {(p) => (
                      <tr>
                        <td>{p.pid}</td>
                        <td>{p.channelCount}</td>
                        <td>{mixerChRange(p.chStart, p.channelCount)}</td>
                        <td><span
                          class={`websrt-pcm ${p.seenPcm ? "on" : ""}`}
                          title={p.seenPcm ? "PCM received" : "No PCM received yet"}
                        >{p.seenPcm ? "●" : "–"}</span></td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </div>

        <div class="websrt-subsection">
          <div class="websrt-subsection-head">
            <span class="websrt-label">PUBLISH</span>
            <span
              class={`websrt-pill ${publishStatus()}`}
              title={`WebSRT publish status: ${publishStatus()}`}
            >{publishStatus()}</span>
          </div>

          <div class="websrt-controls">
            <button
              class={`btn ${pubActive() ? "btn-stop" : "btn-start"}`}
              disabled={publishStatus() === "connecting"}
              onClick={() => (pubActive() ? disconnectPublish() : connectPublish())}
              title={pubActive()
                ? "Stop publishing: closes the master-output tap and the publish worker's SRT/WebTransport session"
                : `Publish the mixer output as SMPTE 302M PCM (48 kHz, no codecs) to the gateway, stream "${publishStreamName()}"`}
            >{pubActive() ? "DISCONNECT" : "CONNECT"}</button>
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
          </div>

          <div
            class="websrt-status-detail"
            title={`Publish stream name and gateway target — same discovery as the receive path (?pubstream / ?host / ?port)`}
          >{publishTarget() ? `stream ${publishStreamName()} → ${publishTarget()}` : `stream ${publishStreamName()} → not connected`}</div>

          <Show when={publishStats()}>
            {(s) => (
              <div
                class="websrt-status-detail"
                title="Publish link stats (last second): TS payload bitrate · SRT round-trip time · cumulative tx-lost packets"
              >{`${s().kbps} kb/s · rtt ${s().rttMs.toFixed(0)} ms · tx loss ${s().txLoss}`}</div>
            )}
          </Show>

          <div class="websrt-status-detail" title={publishStatusDetail()}>
            {publishStatusDetail()}
          </div>
        </div>
      </div>
    </Show>
  );
}
