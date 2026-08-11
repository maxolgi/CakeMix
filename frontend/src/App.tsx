import { onMount, onCleanup, createEffect } from "solid-js";
import { ChannelStrip } from "./components/ChannelStrip";
import { MasterStrip } from "./components/MasterStrip";
import { TransportBar } from "./components/TransportBar";
import {
  channels, setChannels, meterData, setMeterData,
  wasmReady, setWasmReady, isRunning, setIsRunning,
  status, setStatus, setMixerNode, sendToWorklet,
  NUM_CHANNELS,
} from "./stores/mixer";

const SAMPLE_RATE = 48000;

export default function App() {
  let audioCtx: AudioContext | null = null;

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
        if (msg.type === "ready") {
          loadWasm();
        } else if (msg.type === "wasm-ready") {
          setWasmReady(true);
          setStatus("Ready");
        } else if (msg.type === "error") {
          setStatus("Error: " + msg.msg);
        } else if (msg.type === "meter") {
          setMeterData(msg);
        }
      };
    } catch (e: any) {
      setStatus("Init failed: " + e.message);
    }
  });

  onCleanup(() => {
    audioCtx?.close();
  });

  async function loadWasm() {
    try {
      const resp = await fetch("/pkg/mixer_wasm_bg.wasm");
      const wasmBytes = await resp.arrayBuffer();
      sendToWorklet({ type: "init-wasm", wasmBytes });
    } catch (e) {
      setStatus("WASM load failed");
    }
  }

  const start = async () => {
    if (audioCtx?.state === "suspended") await audioCtx.resume();
    sendToWorklet({ type: "start" });
    setIsRunning(true);
  };

  const stop = () => {
    sendToWorklet({ type: "stop" });
    setIsRunning(false);
  };

  return (
    <div class="app">
      <header class="header">
        <div class="brand">
          <h1>🍰 CakeMix</h1>
          <span class="subtitle">WASM Audio Mixer</span>
        </div>
        <div class="header-right">
          <span class={`badge ${wasmReady() ? "active" : ""}`}>
            {wasmReady() ? "WASM ACTIVE" : "WASM LOADING…"}
          </span>
          <span class="status">{status()}</span>
        </div>
      </header>

      <TransportBar
        running={isRunning()}
        wasmReady={wasmReady()}
        meter={meterData()}
        onStart={start}
        onStop={stop}
      />

      <div class="mixer-console">
        {channels.map((_, i) => (
          <ChannelStrip index={i} />
        ))}
        <MasterStrip meter={meterData()} />
      </div>
    </div>
  );
}
