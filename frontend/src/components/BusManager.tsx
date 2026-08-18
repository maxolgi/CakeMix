import { For } from "solid-js";
import { SceneBank } from "./SceneBank";

export function BusManager(props: {
  running: boolean;
  wasmReady: boolean;
  status: string;
  websrtStatus: string;
  expanded: boolean;
  onToggleExpanded: () => void;
  mode: "inputs" | "bus";
  onMode: (m: "inputs" | "bus") => void;
  index: number;
  onIndex: (i: number) => void;
}) {
  // Only transient/error states show in the top bar; "Ready" would be noise.
  const showStatus = () =>
    props.status === "Loading…" ||
    /^(Error|Init failed|WASM load failed)/.test(props.status);

  return (
    <div class="bus-manager" title="Buses">
      <button
        class="websrt-chevron"
        onClick={() => props.onToggleExpanded()}
        title={props.expanded ? "Collapse settings" : "Expand settings (engine, test tones, WebSRT)"}
      >{props.expanded ? "▾" : "▸"}</button>
      <div class="view-selector">
        <button
          class={`mode-toggle ${props.mode === "bus" ? "bus" : ""}`}
          onClick={() => props.onMode(props.mode === "inputs" ? "bus" : "inputs")}
          title={props.mode === "inputs" ? "Switch to bus view" : "Switch to inputs view"}
        >{props.mode === "inputs" ? "INPUTS" : "BUS"}</button>
        <div class="bank-selector">
          <For each={[0, 1, 2, 3, 4, 5, 6, 7]}>
            {(b) => (
              <button
                class={`bank-btn ${props.index === b ? "active" : ""}`}
                onClick={() => props.onIndex(b)}
                title={props.mode === "inputs"
                  ? `Channels ${b * 16 + 1}–${(b + 1) * 16}`
                  : `Bus ${b + 1}`}
              >{b + 1}</button>
            )}
          </For>
        </div>
      </div>
      <SceneBank />
      <span
        class={`status-text ${/^(Error|Init failed|WASM load failed)/.test(props.status) ? "err" : ""}`}
        style={showStatus() ? {} : { display: "none" }}
        title={props.status}
      >{props.status}</span>
      <span
        class={`websrt-pill ${props.websrtStatus}`}
        title={`WebSRT connection status: ${props.websrtStatus}`}
      >{props.websrtStatus}</span>
    </div>
  );
}
