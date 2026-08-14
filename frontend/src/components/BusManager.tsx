import { For, createSignal } from "solid-js";
import { buses, addBus } from "../stores/mixer";

const BUS_TYPE_LABELS = ["Group", "Aux", "Matrix"];

export function BusManager(props: {
  running: boolean;
  wasmReady: boolean;
  onStart: () => void;
  onStop: () => void;
  bank: number;
  onBank: (n: number) => void;
}) {
  const onAdd = () => {
    const n = buses().length + 1;
    addBus(`Bus ${n}`, 1);
  };

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
      <span class="bus-manager-label">BUSES</span>
      <div class="bus-manager-list">
        <For each={buses()}>
          {(bus) => (
            <span class="bus-item" title={`${bus.name} (${BUS_TYPE_LABELS[bus.type] ?? "?"})`}>
              <span class="bus-item-id">#{bus.id}</span>
              <span class="bus-item-name">{bus.name}</span>
              <span class="bus-item-type">{BUS_TYPE_LABELS[bus.type] ?? "?"}</span>
            </span>
          )}
        </For>
        {buses().length === 0 && <span class="bus-manager-empty">No buses</span>}
      </div>
      <button class="bus-add-btn" onClick={onAdd} title="Create a new Auxiliary bus">+ BUS</button>
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
