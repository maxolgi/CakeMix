import { onMount, onCleanup, createSignal, createEffect, For } from "solid-js";
import * as quicsrt from "./quicsrt/client";
import * as quicsrtStore from "./quicsrt/store";
import * as scenesStore from "./stores/scenes";
import { quicsrtStatus } from "./quicsrt/store";
import { publishStatus, relayPubPcm } from "./quicsrt/publish";
import { registerAudioUnlock } from "./audio/unlock";
import { ChannelDetailPanel } from "./components/ChannelDetailPanel";
import { MasterStrip } from "./components/MasterStrip";
import { BusMasterStrip } from "./components/BusMasterStrip";
import { BusManager } from "./components/BusManager";
import { QuicSRTPanel } from "./components/QuicSRTPanel";
import {
  updateMeterData, applyConsoleParams,
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
  const [settingsExpanded, setSettingsExpanded] = createSignal(false);

  onMount(async () => {
    // Build plumbing: expose the QuicSRT client (receive worker bootstrap)
    // and the QuicSRT store (connect/disconnect + status signals) on window
    // so the bundler keeps the worker chunk and store in the
    // bundle. Also reachable from the console for manual testing; connection
    // itself is user-triggered (settings drawer).
    (window as any).__cakemix_quicsrt = quicsrt;
    (window as any).__cakemix_quicsrt_store = quicsrtStore;

    // Autoplay unlock: connect click handlers call this synchronously so
    // resume() traces to the user gesture even across the async handshake.
    registerAudioUnlock(() => { audioCtx?.resume(); });

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
        else if (msg.type === "meter") {
          updateMeterData(msg);
          // Cumulative worklet-side pcm drop count rides the meter tick.
          if (typeof msg.droppedPcm === "number") quicsrtStore.onWorkletPcmDropped(msg.droppedPcm);
        }
        else if (msg.type === "pid-mapped") { quicsrtStore.onWorkletPidMapped(msg); }
        else if (msg.type === "scene-saved") { scenesStore.addScene(msg.id); }
        // Scene-recall UI loop: a recall happened in the mixer — pull the
        // console state back so sliders/knobs reflect it (otherwise the
        // next interaction writes stale values over the recalled scene).
        else if (msg.type === "scene-recalled") { sendToWorklet({ type: "get-params" }); }
        else if (msg.type === "console-params") { applyConsoleParams(msg.json); }
        else if (msg.type === "pcm-dropped") { quicsrtStore.onWorkletPcmDropped(msg.total); }
        else if (msg.type === "pub-pcm") { relayPubPcm(msg.samples, msg.ptsUs, msg.channels); }
      };
    } catch (e: any) { setStatus("Init failed: " + e.message); }
  });

  // Engine auto-start: any active transport (receive or publish) needs the
  // mixer running; starting manually is still possible from the drawer.
  createEffect(() => {
    if (quicsrtStatus() === "connected" || publishStatus() === "connected") {
      if (wasmReady() && !isRunning()) {
        sendToWorklet({ type: "start" });
        setIsRunning(true);
      }
    }
  });

  onCleanup(() => { quicsrtStore.disconnectQuicsrt(); audioCtx?.close(); });

  async function loadWasm() {
    try {
      const resp = await fetch("/pkg/mixer_wasm_bg.wasm");
      const wasmBytes = await resp.arrayBuffer();
      sendToWorklet({ type: "init-wasm", wasmBytes });
    } catch (e) { setStatus("WASM load failed"); }
  }

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
        quicsrtStatus={quicsrtStatus()}
        expanded={settingsExpanded()}
        onToggleExpanded={() => setSettingsExpanded(!settingsExpanded())}
        mode={mode()}
        onMode={onMode}
        index={mode() === "bus" ? (selectedBus() ?? 0) : bank()}
        onIndex={onSelectIndex}
      />
      <QuicSRTPanel expanded={settingsExpanded()} />
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
