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

const LATENCY_OPTIONS = [120, 250, 500, 1000, 2000];

const mixerChRange = (chStart: number, channelCount: number) =>
  channelCount > 1 ? `${chStart + 1}–${chStart + channelCount}` : `${chStart + 1}`;

export function WebSRTPanel() {
  const [expanded, setExpanded] = createSignal(false);

  const active = () => {
    const s = websrtStatus();
    return s === "connected" || s === "connecting";
  };

  return (
    <div class="websrt-panel">
      <div class="websrt-header">
        <span class="websrt-label">WEBSRT</span>
        <span
          class={`websrt-pill ${websrtStatus()}`}
          title={`WebSRT connection status: ${websrtStatus()}`}
        >{websrtStatus()}</span>
        <button
          class="websrt-chevron"
          onClick={() => setExpanded(!expanded())}
          title={expanded() ? "Collapse WebSRT panel" : "Expand WebSRT panel"}
        >{expanded() ? "▾" : "▸"}</button>
      </div>

      {expanded() && (
        <div class="websrt-body">
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
      )}
    </div>
  );
}
