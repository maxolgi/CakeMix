import { createSignal } from "solid-js";
import { sendToWorklet } from "./mixer";

// Scene bank state. The wasm only exposes save/recall/delete + scene_count —
// no id getter — so the UI tracks ids itself from the worklet's scene-saved
// events. Simple in-memory state; nothing persists across reloads (the wasm
// scenes die with the worklet anyway).

export interface SceneEntry { id: number; label: string; savedAt: number; }

export const [scenes, setScenes] = createSignal<SceneEntry[]>([]);
export const [activeScene, setActiveScene] = createSignal<number | null>(null);

// Worklet → main: a save completed and the wasm assigned this id
// (ids start at 1 and increment — the label mirrors that).
export function addScene(id: number) {
  setScenes([...scenes(), { id, label: `Scene ${id}`, savedAt: Date.now() }]);
}

export function removeScene(id: number) {
  setScenes(scenes().filter((s) => s.id !== id));
  if (activeScene() === id) setActiveScene(null);
}

// Store helpers — each sends the worklet message (mixer.ts convention)
export function sendSceneSave() {
  sendToWorklet({ type: "scene-save" });
}
export function sendSceneRecall(id: number, fadeMs: number) {
  sendToWorklet({ type: "scene-recall", id, fadeMs });
  setActiveScene(id);
}
export function sendSceneDelete(id: number) {
  sendToWorklet({ type: "scene-delete", id });
}
