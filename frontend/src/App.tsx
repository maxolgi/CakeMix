import { onMount, onCleanup, createSignal, createEffect, For, Show } from "solid-js";
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
  updateMeterData, applyConsoleParams, applyFxCatalog, applyFxError, applyFxOut, requestFxCatalog, applySideMapped,
  fxLastError, setFxLastError,
  wasmReady, setWasmReady, isRunning, setIsRunning,
  status, setStatus, setMixerNode, sendToWorklet,
  selectedBus, setSelectedBus, BUS_STEREO_SLOTS,
} from "./stores/mixer";

const SAMPLE_RATE = 48000;
/** Stereo channels per bank: 64 stereo channels → 4 banks (16 each). */
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
        else if (msg.type === "wasm-ready") {
          setWasmReady(true);
          setStatus("Ready");
          // One-shot pulls once the wasm can answer (same lifecycle point
          // as every other initial request): the FX catalog drives the FX
          // rack UI. The worklet ALWAYS replies, even without the fx API.
          requestFxCatalog();
        }
        else if (msg.type === "error") { setStatus("Error: " + msg.msg); console.error("worklet:", msg.msg); }
        else if (msg.type === "meter") {
          updateMeterData(msg);
          // Cumulative worklet-side pcm drop count rides the meter tick.
          if (typeof msg.droppedPcm === "number") quicsrtStore.onWorkletPcmDropped(msg.droppedPcm);
        }
        else if (msg.type === "pid-mapped") {
          quicsrtStore.onWorkletPidMapped(msg);
          // Mirror the mapping into the stereo store's sideMapped flags:
          // ch range [chStart, chStart+count). chStart -1 = capped (PCM
          // dropped worklet-side) → nothing to mark. Unmapped sessions
          // never echo (see applySideMapped's note).
          if (typeof msg.chStart === "number" && msg.chStart >= 0) {
            for (let ch = msg.chStart; ch < msg.chStart + msg.channelCount; ch++) {
              applySideMapped(ch, true);
            }
          }
        }
        else if (msg.type === "scene-saved") { scenesStore.addScene(msg.id); }
        // Scene-recall UI loop: a recall happened in the mixer — pull the
        // console state back so sliders/knobs reflect it (otherwise the
        // next interaction writes stale values over the recalled scene).
        else if (msg.type === "scene-recalled") { sendToWorklet({ type: "get-params" }); }
        else if (msg.type === "console-params") { applyConsoleParams(msg.json); }
        else if (msg.type === "fx-catalog") { applyFxCatalog(msg.json); }
        // An fx op the engine rejected (instance cap, unavailable effect):
        // the store rolls its optimistic slot back and raises the toast.
        else if (msg.type === "fx-error") { applyFxError(msg); }
        // FX out-port pull reply (the tuner readout polls get-fx-out):
        // display-only, latest reply wins in the store.
        else if (msg.type === "fx-out") { applyFxOut(msg); }
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

  // FX rejection toast lifetime: auto-clear the transient error ~4s after
  // it lands. A newer rejection restarts the window — the effect re-runs,
  // so the cleanup below clears the previous timer first.
  createEffect(() => {
    if (!fxLastError()) return;
    const t = setTimeout(() => setFxLastError(null), 4000);
    onCleanup(() => clearTimeout(t));
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
          // Inputs view: 64 stereo channels (bank of 16; channel t = engine
          // strips 2t/2t+1 — the panel fans every control out to both).
          <For each={Array.from({ length: STRIPS_PER_BANK }, (_, i) => bank() * STRIPS_PER_BANK + i)}>
            {(t) => <ChannelDetailPanel channelIndex={t} stereo />}
          </For>
        ) : (
          // Bus view: 8 stereo slot strips per bus (stereo slot t = this
          // bus's mono slot strips 2t/2t+1 — the panel fans every control
          // out to both, same as the input strips).
          <For each={Array.from({ length: BUS_STEREO_SLOTS }, (_, s) => s)}>
            {(t) => (
              <ChannelDetailPanel slot={{ bus: selectedBus()!, slot: t }} />
            )}
          </For>
        )}
        {selectedBus() !== null && <BusMasterStrip bus={selectedBus()!} />}
        <MasterStrip />
      </div>
      {/* FX rejection toast (transient — see the fxLastError effect above) */}
      <Show when={fxLastError()}>
        {(err) => (
          <div class="fx-error-toast" title={err().msg}>
            FX change rejected — {err().msg}
          </div>
        )}
      </Show>
    </div>
  );
}
