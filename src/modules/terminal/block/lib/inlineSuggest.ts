import { Prec, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  keymap,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { type Offer, offerFor } from "@/modules/terminal/suggest/engine";
import type { Suggestion } from "./history";

const setSuggestion = StateEffect.define<Suggestion | null>();

const suggestionField = StateField.define<Suggestion | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setSuggestion)) return e.value;
    if (tr.docChanged && value) {
      // Kept only while it continues what's typed; a correction that
      // replaces the line is fetched again for the new text.
      const doc = tr.state.doc.toString();
      return value.text.length > doc.length && value.text.startsWith(doc)
        ? value
        : null;
    }
    return value;
  },
});

class GhostWidget extends WidgetType {
  constructor(
    private readonly text: string,
    private readonly fix: boolean,
  ) {
    super();
  }
  eq(other: GhostWidget) {
    return other.text === this.text && other.fix === this.fix;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = this.fix ? "cm-ghost cm-ghost-fix" : "cm-ghost";
    span.textContent = this.text;
    return span;
  }
  ignoreEvent() {
    return false;
  }
}

function offer(state: EditorView["state"]): Offer | null {
  const sugg = state.field(suggestionField, false);
  if (!sugg) return null;
  const sel = state.selection.main;
  if (!sel.empty || sel.head !== state.doc.length) return null;
  const doc = state.doc.toString();
  // An empty line shows only a correction offered after a failure.
  if (doc.length === 0 && !sugg.fix) return null;
  return offerFor(doc, sugg);
}

const ghostDecorations = EditorView.decorations.compute(
  [suggestionField, "doc", "selection"],
  (state): DecorationSet => {
    const o = offer(state);
    if (o === null) return Decoration.none;
    return Decoration.set([
      Decoration.widget({
        widget: new GhostWidget(o.draw, o.fix),
        side: 1,
      }).range(state.doc.length),
    ]);
  },
);

export function acceptInlineSuggestion(view: EditorView): boolean {
  const o = offer(view.state);
  if (o === null) return false;
  const end = view.state.doc.length;
  const from = o.replace ? 0 : end;
  view.dispatch({
    changes: { from, to: end, insert: o.text },
    selection: { anchor: from + o.text.length },
    effects: setSuggestion.of(null),
  });
  return true;
}

/** Offer the correction of a command that just failed, on an empty input. */
export function offerFix(view: EditorView, text: string): void {
  if (view.state.doc.length > 0) return;
  view.dispatch({ effects: setSuggestion.of({ text, fix: true }) });
}

function fetcherPlugin(fetch: (line: string) => Promise<Suggestion | null>) {
  return ViewPlugin.fromClass(
    class {
      private timer: ReturnType<typeof setTimeout> | null = null;
      update(update: ViewUpdate) {
        if (!update.docChanged) return;
        if (this.timer) clearTimeout(this.timer);
        const view = update.view;
        const line = view.state.doc.toString();
        if (!line) return;
        // A correction still being typed out stays; history waits.
        const kept = view.state.field(suggestionField, false);
        if (kept?.fix) return;
        this.timer = setTimeout(() => {
          if (view.state.doc.toString() !== line) return;
          fetch(line)
            .then((sugg) => {
              if (sugg && view.state.doc.toString() === line) {
                view.dispatch({ effects: setSuggestion.of(sugg) });
              }
            })
            .catch(() => {});
        }, 70);
      }
      destroy() {
        if (this.timer) clearTimeout(this.timer);
      }
    },
  );
}

export function inlineSuggestion(
  fetch: (line: string) => Promise<Suggestion | null>,
) {
  return [
    suggestionField,
    ghostDecorations,
    fetcherPlugin(fetch),
    Prec.highest(
      keymap.of([
        { key: "ArrowRight", run: acceptInlineSuggestion },
        { key: "End", run: acceptInlineSuggestion },
        { key: "Mod-f", run: acceptInlineSuggestion },
      ]),
    ),
  ];
}
