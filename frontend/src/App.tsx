import { onMount, onCleanup, createSignal, For } from "solid-js";
import * as websrt from "./websrt/client";
import * as websrtStore from "./websrt/store";
import { ChannelDetailPanel } from "./components/ChannelDetailPanel";
import { MasterStrip } from "./components/MasterStrip";
import { BusMasterStrip } from "./components/BusMasterStrip";
import { BusManager } from "./components/BusManager";
import { WebSRTPanel } from "./components/WebSRTPanel";
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
  const [mode, setMode] = createSignal<"inputs" | "bus">("inputs");
  const [bank, setBank] = createSignal(0);

  onMount(async () => {
    // Build plumbing: expose the WebSRT client (receive worker + muxer init)
    // and the WebSRT store (connect/disconnect + status signals) on window
    // so the bundler keeps the worker chunk, wasm binaries and store in the
    // bundle. Also reachable from the console for manual testing; connection
    // itself is user-triggered (WebSRTPanel).
    (window as any).__cakemix_websrt = websrt;
    (window as any).__cakemix_websrt_store = websrtStore;

    try {
      audioCtx = new AudioContext({ sampleRate: SAMPLE_RATE });
      if (!audioCtx.audioWorklet) {
        setStatus("Init failed: AudioWorklet unavailable — secure context required. Use HTTPS or localhost.");
        return;
      }
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
        else if (msg.type === "error") { setStatus("Error: " + msg.msg); console.error("worklet:", msg.msg); }
        else if (msg.type === "meter") { updateMeterData(msg); }
      };
    } catch (e: any) { setStatus("Init failed: " + e.message); }
  });

  onCleanup(() => { websrtStore.disconnectWebsrt(); audioCtx?.close(); });

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

  const onSelectIndex = (i: number) => {
    if (mode() === "inputs") { setBank(i); }
    else { setSelectedBus(i); }
  };

  const onMode = (m: "inputs" | "bus") => {
    setMode(m);
    if (m === "inputs") { setSelectedBus(null); }
    else { setSelectedBus(selectedBus() ?? bank()); }
  };

  return (
    <div class="app">
      <BusManager
        running={isRunning()}
        wasmReady={wasmReady()}
        status={status()}
        onStart={start}
        onStop={stop}
        mode={mode()}
        onMode={onMode}
        index={mode() === "bus" ? (selectedBus() ?? 0) : bank()}
        onIndex={onSelectIndex}
      />
      <WebSRTPanel />
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
