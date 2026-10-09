import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Seg } from "./agreement";
import type { ModelId } from "./text";

/** The built-in Whistle model's commands (src-tauri/src/modules/stt.rs). */

type Progress = { model: string; received: number; total: number };

export async function modelReady(model: ModelId): Promise<boolean> {
  const s = await invoke<{ ready: boolean; bytes: number }>(
    "stt_model_status",
    {
      model,
    },
  );
  return s.ready;
}

export async function downloadModel(
  model: ModelId,
  onPct: (pct: number) => void,
): Promise<void> {
  const unlisten = await listen<Progress>("stt://download", (e) => {
    if (e.payload.model !== model || e.payload.total <= 0) return;
    onPct(Math.floor((e.payload.received * 100) / e.payload.total));
  });
  try {
    await invoke("stt_download_model", { model });
  } finally {
    unlisten();
  }
}

export function removeModel(model: ModelId): Promise<void> {
  return invoke("stt_remove_model", { model });
}

/** A live pass: 16 kHz mono samples in as raw bytes (no JSON), phrases with times out. */
export async function transcribeLive(
  model: ModelId,
  samples: Float32Array,
): Promise<Seg[]> {
  if (samples.length === 0) return [];
  const bytes = new Uint8Array(
    samples.buffer,
    samples.byteOffset,
    samples.byteLength,
  );
  return invoke<Seg[]>("stt_transcribe_live", bytes, {
    headers: { "x-stt-model": model },
  });
}

/** Dictation switched on: load the model so the first pass is quick. */
export function loadModel(model: ModelId): Promise<void> {
  return invoke("stt_load", { model });
}
