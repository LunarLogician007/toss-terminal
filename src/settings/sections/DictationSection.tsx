import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  downloadModel,
  modelReady,
  removeModel,
} from "@/modules/dictation/lib/builtin";
import { MODEL } from "@/modules/dictation/lib/text";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * TOSS Terminal: local dictation. Whistle runs inside TOSS Terminal; the model
 * is downloaded once and nothing you say leaves your computer.
 */
export function DictationSection() {
  const [ready, setReady] = useState<boolean | null>(null);
  const [pct, setPct] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The status check failed (no engine on this platform): nothing to download.
  const [unavailable, setUnavailable] = useState(false);

  useEffect(() => {
    let live = true;
    modelReady(MODEL.id)
      .then((r) => live && setReady(r))
      .catch((e) => {
        if (!live) return;
        setReady(false);
        setUnavailable(true);
        setError(errorText(e));
      });
    return () => {
      live = false;
    };
  }, []);

  const download = async () => {
    setError(null);
    setPct(0);
    try {
      await downloadModel(MODEL.id, setPct);
      setReady(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setPct(null);
    }
  };

  const remove = async () => {
    setError(null);
    try {
      await removeModel(MODEL.id);
      setReady(false);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const status =
    pct !== null
      ? `Downloading… ${pct}%`
      : ready === null
        ? "Checking…"
        : ready
          ? "Ready"
          : "Not downloaded";

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="Dictation"
        description='Speak into the terminal. Turn it on with "mic" in the status bar, then press the prefix and Ctrl+Space (or v). Whistle by Cactus Compute runs on the CPU inside TOSS Terminal; nothing you say leaves your computer.'
      />

      <div className="flex flex-col gap-2">
        <SettingRow
          title={`Speech model: Whistle, ${MODEL.mb} MB`}
          description={error ?? status}
        >
          {ready ? (
            <Button
              variant="outline"
              size="sm"
              className="h-8 px-2.5 text-[11px]"
              onClick={() => void remove()}
            >
              Remove
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              disabled={pct !== null || ready === null || unavailable}
              className="h-8 px-2.5 text-[11px]"
              onClick={() => void download()}
            >
              Download
            </Button>
          )}
        </SettingRow>
      </div>
    </div>
  );
}
