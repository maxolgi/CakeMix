import { For } from "solid-js";

export function BusManager(props: {
  running: boolean;
  wasmReady: boolean;
  onStart: () => void;
  onStop: () => void;
  bank: number;
  onBank: (n: number) => void;
  selectedBus: number | null;
  onSelectBus: (b: number | null) => void;
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
      <div class="bus-selector">
        <span class="bus-manager-label">BUS</span>
        <For each={[0, 1, 2, 3, 4, 5, 6, 7]}>
          {(b) => (
            <button
              class={`bank-btn ${props.selectedBus === b ? "active" : ""}`}
              onClick={() => props.selectedBus === b ? props.onSelectBus(null) : props.onSelectBus(b)}
              title={`Bus ${b + 1}`}
            >{b + 1}</button>
          )}
        </For>
      </div>
      <div class="bank-selector">
        <For each={[0, 1, 2, 3, 4, 5, 6, 7]}>
          {(b) => (
            <button
              class={`bank-btn ${props.bank === b ? "active" : ""}`}
              onClick={() => props.onBank(b)}
              title={`Channels ${b * 16 + 1}–${(b + 1) * 16}`}
            >{b + 1}</button>
          )}
        </For>
      </div>
    </div>
  );
}
