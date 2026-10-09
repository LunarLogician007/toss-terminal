// Modified for TOSS Terminal, 2026: no motion; the title bar drags the window.

import { cn } from "@/lib/utils";
import type { ReactNode } from "react";
import type { Rect } from "./lib/layout";

type Props = {
  rect: Rect;
  hidden: boolean;
  focused: boolean;
  titleBar: boolean;
  dim: boolean;
  title: string;
  badge: ReactNode;
  onClose?: () => void;
  onZoom?: () => void;
  children?: ReactNode;
};

/**
 * One pane drawn as a tuios-style window: a frame, a thin title bar with
 * close and zoom marks, and the accent colour when focused. The title bar
 * also moves the window, as there is no other title bar to grab.
 */
export function TileWindow(props: Props) {
  const { rect, hidden, focused, titleBar, dim } = props;
  return (
    <div
      className={cn(
        "absolute flex flex-col overflow-hidden border bg-background",
        focused ? "border-[var(--sidebar-primary)]" : "border-border",
        hidden && "pointer-events-none",
      )}
      style={{
        left: rect.x,
        top: rect.y,
        width: rect.width,
        height: rect.height,
        opacity: hidden ? 0 : 1,
      }}
      data-tile-focused={focused || undefined}
    >
      {titleBar && (
        <div
          data-tauri-drag-region
          className={cn(
            "flex h-5 shrink-0 select-none items-center gap-2 border-b px-1.5 font-mono text-[11px]",
            focused
              ? "border-[var(--sidebar-primary)]/40 text-[var(--sidebar-primary)]"
              : "border-border text-muted-foreground",
          )}
        >
          <button
            type="button"
            aria-label="Close pane"
            title="Close pane"
            onClick={props.onClose}
            className="hover:text-foreground"
          >
            [x]
          </button>
          <button
            type="button"
            aria-label="Zoom pane"
            title="Zoom pane"
            onClick={props.onZoom}
            className="hover:text-foreground"
          >
            [z]
          </button>
          <span data-tauri-drag-region className="min-w-0 flex-1 truncate">
            {props.title}
          </span>
          {props.badge}
        </div>
      )}
      <div
        className="relative min-h-0 flex-1"
        style={{ opacity: dim && !focused ? 0.85 : 1 }}
      >
        {props.children}
      </div>
    </div>
  );
}
