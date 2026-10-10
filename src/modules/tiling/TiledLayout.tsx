import { usePreferencesStore } from "@/modules/settings/preferences";
import { useAgentActivityStore } from "@/modules/terminal/lib/agentActivity";
import {
  findLeafCwd,
  type PaneId,
  type PaneNode,
} from "@/modules/terminal/lib/panes";
import { ptyIdForLeaf } from "@/modules/terminal/lib/useTerminalSession";
import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTilingActionsStore } from "./lib/actionsStore";
import { type Divider, layoutTiles } from "./lib/layout";
import { useTilingLayoutStore } from "./lib/layoutStore";
import {
  beginTerminalResizeInteraction,
  endTerminalResizeInteraction,
} from "./lib/resizeHold";
import { planTiles } from "./lib/tilePlan";
import { TileWindow } from "./TileWindow";

type Props = {
  tabId: number;
  node: PaneNode;
  activeLeafId: number;
  zoomedLeafId?: PaneId;
  /** The terminal (and its overlays) for one pane. */
  renderLeaf: (leafId: PaneId, focused: boolean) => ReactNode;
};

function basename(path: string | undefined): string {
  if (!path) return "terminal";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

function AgentBadge({ leafId }: { leafId: PaneId }) {
  const ptyId = ptyIdForLeaf(leafId);
  const phase = useAgentActivityStore((s) =>
    ptyId === null ? undefined : s.phases[ptyId],
  );
  const agent = useAgentActivityStore((s) =>
    ptyId === null ? undefined : s.agents[ptyId],
  );
  if (!phase || phase === "idle") return null;
  const mark = phase === "working" ? "◐" : phase === "attention" ? "●" : "✓";
  const label =
    phase === "working"
      ? "working"
      : phase === "attention"
        ? "needs you"
        : "done";
  return (
    <span
      className="shrink-0 font-mono text-[10px]"
      title={`${agent ?? "agent"}: ${label}`}
    >
      {mark} {agent ?? label}
    </span>
  );
}

/**
 * A terminal tab's panes, tiled the tuios way: each pane is a window placed
 * by layoutTiles, drawn as one flat list keyed by pane id, so a change in the
 * tree moves windows instead of recreating terminals.
 */
export function TiledLayout({
  tabId,
  node,
  activeLeafId,
  zoomedLeafId,
  renderLeaf,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const width = Math.round(el.clientWidth);
      const height = Math.round(el.clientHeight);
      setSize((s) =>
        s.width === width && s.height === height ? s : { width, height },
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const gap = usePreferencesStore((s) => s.tilingGap);
  const titleBars = usePreferencesStore((s) => s.tilingTitleBars);
  const dim = usePreferencesStore((s) => s.tilingDimUnfocused);
  const actions = useTilingActionsStore((s) => s.actions);

  const { tiles, dividers } = useMemo(
    () =>
      layoutTiles(
        node,
        { x: 0, y: 0, width: size.width, height: size.height },
        gap,
        zoomedLeafId,
      ),
    [node, size, gap, zoomedLeafId],
  );

  const ready = size.width > 0 && size.height > 0;
  const items = planTiles(tiles, activeLeafId);

  useEffect(() => {
    if (!ready) return;
    useTilingLayoutStore
      .getState()
      .setLayout(tabId, { tiles, dividers, gap, ...size });
  }, [tabId, tiles, dividers, gap, size, ready]);
  useEffect(() => () => useTilingLayoutStore.getState().clear(tabId), [tabId]);

  return (
    <div ref={ref} className="relative h-full w-full overflow-hidden">
      {ready &&
        items.map((item) => (
          <TileWindow
            key={item.key}
            rect={item.rect}
            hidden={item.hidden}
            focused={item.focused}
            titleBar={titleBars}
            dim={dim}
            title={basename(findLeafCwd(node, item.id))}
            badge={<AgentBadge leafId={item.id} />}
            onClose={() => actions.closeLeaf(item.id)}
            onZoom={() => actions.toggleZoom(tabId, item.id)}
          >
            {renderLeaf(item.id, item.focused)}
          </TileWindow>
        ))}
      {ready &&
        dividers.map((d) => (
          <DividerHandle
            key={`${d.splitId}:${d.index}`}
            divider={d}
            onDrag={(deltaPx) => actions.adjustDivider(tabId, d, deltaPx)}
            onReset={() => actions.resetDivider(tabId, d.splitId)}
          />
        ))}
    </div>
  );
}

const MIN_HANDLE_PX = 8;

function DividerHandle({
  divider,
  onDrag,
  onReset,
}: {
  divider: Divider;
  onDrag: (deltaPx: number) => void;
  onReset: () => void;
}) {
  const row = divider.dir === "row";
  const { rect } = divider;
  const thickness = Math.max(MIN_HANDLE_PX, row ? rect.width : rect.height);
  const offset = ((row ? rect.width : rect.height) - thickness) / 2;
  const last = useRef<number | null>(null);
  const token = useRef({});

  const stop = () => {
    if (last.current === null) return;
    last.current = null;
    endTerminalResizeInteraction(token.current);
  };

  return (
    <div
      aria-hidden
      className={
        row ? "absolute cursor-col-resize" : "absolute cursor-row-resize"
      }
      style={
        row
          ? {
              left: rect.x + offset,
              top: rect.y,
              width: thickness,
              height: rect.height,
            }
          : {
              left: rect.x,
              top: rect.y + offset,
              width: rect.width,
              height: thickness,
            }
      }
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        last.current = row ? e.clientX : e.clientY;
        beginTerminalResizeInteraction(token.current);
      }}
      onPointerMove={(e) => {
        if (last.current === null) return;
        const now = row ? e.clientX : e.clientY;
        const delta = now - last.current;
        if (delta === 0) return;
        last.current = now;
        onDrag(delta);
      }}
      onPointerUp={stop}
      onPointerCancel={stop}
      onLostPointerCapture={stop}
      onDoubleClick={onReset}
    />
  );
}
