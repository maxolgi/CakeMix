import { onMount, onCleanup, createSignal, For } from "solid-js";
import { ChannelDetailPanel } from "./components/ChannelDetailPanel";
import { MasterStrip } from "./components/MasterStrip";
import { BusMasterStrip } from "./components/BusMasterStrip";
import { BusManager } from "./components/BusManager";
import {
  updateMeterData,
  wasmReady, setWasmReady, isRunning, setIsRunning,
  status, setStatus, setMixerNode, sendToWorklet,
  selectedBus, setSelectedBus, slotChannelIndex,
} from "./stores/mixer";

const SAMPLE_RATE = 48000;
const STRIPS_PER_BANK = 16;

export default function App() {
  let audioCtx: AudioContext | null = null;
  const [bank, setBank] = createSignal(0);

  onMount(async () => {
    try {
      audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
      await audioCtx.audioWorklet.addModule("/mixer-worklet-processor.js");
      const node = new AudioWorkletNode(audioCtx, "mixer-processor", {
        numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2],
      });
      node.connect(audioCtx.destination);
      setMixerNode(node);

      node.port.onmessage = (e: MessageEvent) => {
        const msg = e.data;
        if (msg.type === "ready") { loadWasm(); }
        else if (msg.type === "wasm-ready") { setWasmReady(true); setStatus("Ready"); }
        else if (msg.type === "error") { setStatus("Error: " + msg.msg); }
        else if (msg.type === "meter") { updateMeterData(msg); }
      };
    } catch (e: any) { setStatus("Init failed: " + e.message); }
  });

  onCleanup(() => { audioCtx?.close(); });

  async function loadWasm() {
    try {
      const resp = await fetch("/pkg/mixer_wasm_bg.wasm");
      const wasmBytes = await resp.arrayBuffer();
      sendToWorklet({ type: "init-wasm", wasmBytes });
    } catch (e) { setStatus("WASM load failed"); }
  }

  const start = async () => {
    if (audioCtx?.state === "suspended") await audioCtx.resume();
    sendToWorklet({ type: "start" });
    setIsRunning(true);
  };
  const stop = () => { sendToWorklet({ type: "stop" }); setIsRunning(false); };

  return (
    <div class="app">
      <BusManager
        running={isRunning()}
        wasmReady={wasmReady()}
        onStart={start}
        onStop={stop}
        bank={bank()}
        onBank={setBank}
        selectedBus={selectedBus()}
        onSelectBus={setSelectedBus}
      />
      <div class="mixer-console">
        {selectedBus() === null ? (
          <For each={Array.from({ length: STRIPS_PER_BANK }, (_, i) => bank() * STRIPS_PER_BANK + i)}>
            {(chIdx) => <ChannelDetailPanel channelIndex={chIdx} />}
          </For>
        ) : (
          <For each={Array.from({ length: 16 }, (_, s) => s)}>
            {(s) => (
              <ChannelDetailPanel
                channelIndex={slotChannelIndex(selectedBus()!, s)}
                slot={{ bus: selectedBus()!, slot: s }}
              />
            )}
          </For>
        )}
        {selectedBus() !== null && <BusMasterStrip bus={selectedBus()!} />}
        <MasterStrip />
      </div>
    </div>
  );
}
