import { createSignal, For } from "solid-js";
import {
  scenes, activeScene,
  sendSceneSave, sendSceneRecall, sendSceneDelete, removeScene,
} from "../stores/scenes";

const FADE_DEFAULT = 500;
const FADE_MIN = 0;
const FADE_MAX = 10000;

/** Scene bank for the top bar: SAVE, recall cross-fade time, one chip per
 *  saved scene. Scene ids live in the scenes store (fed by the worklet's
 *  scene-saved events); the wasm holds the captured console state. */
export function SceneBank() {
  const [fadeMs, setFadeMs] = createSignal(FADE_DEFAULT);

  const onFadeInput = (raw: string) => {
    const ms = parseInt(raw, 10);
    if (Number.isFinite(ms)) setFadeMs(Math.min(FADE_MAX, Math.max(FADE_MIN, ms)));
  };

  return (
    <div class="scene-bank">
      <button
        class="scene-save-btn"
        onClick={sendSceneSave}
        title="Save a scene: capture every strip, bus and master parameter (gain, pan, EQ, dynamics, routing, mutes) into a new scene held by the mixer engine."
      >SAVE</button>
      <label class="detail-select-label">FADE
        <input
          class="scene-fade-input"
          type="number"
          min={FADE_MIN}
          max={FADE_MAX}
          value={String(fadeMs())}
          onInput={(e) => onFadeInput(e.currentTarget.value)}
          title={`Scene recall cross-fade in ms (${FADE_MIN}–${FADE_MAX}). 0 = instant recall; otherwise every recalled parameter fades from the current console state over this duration. Applies to the next chip click.`}
        />
      </label>
      <div class="scene-chips">
        <For each={scenes()}>
          {(s) => (
            <button
              class={`scene-chip ${activeScene() === s.id ? "active" : ""}`}
              onClick={() => sendSceneRecall(s.id, fadeMs())}
              title={`Recall ${s.label} (id ${s.id}) with a ${fadeMs()} ms cross-fade`}
            >{s.label}<span
                class="scene-chip-x"
                onClick={(e) => { e.stopPropagation(); sendSceneDelete(s.id); removeScene(s.id); }}
                title={`Delete ${s.label} (id ${s.id}) from the scene bank`}
              >×</span></button>
          )}
        </For>
      </div>
    </div>
  );
}
