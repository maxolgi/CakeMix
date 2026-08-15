import { For } from "solid-js";

export function BusManager(props: {
  running: boolean;
  wasmReady: boolean;
  status: string;
  onStart: () => void;
  onStop: () => void;
  mode: "inputs" | "bus";
  onMode: (m: "inputs" | "bus") => void;
  index: number;
  onIndex: (i: number) => void;
}) {
  return (
    <div class="bus-manager" title="Buses">
      <button
        class="btn btn-start"
        disabled={!props.wasmReady || props.running}
        onClick={props.onStart}
      >Start</button>
      <button
        class="btn btn-stop"
        disabled={!props.running}
        onClick={props.onStop}
      >Stop</button>
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
      <span
        class={`status-text ${/^(Error|Init failed|WASM load failed)/.test(props.status) ? "err" : ""}`}
        title={props.status}
      >{props.status}</span>
    </div>
  );
}
