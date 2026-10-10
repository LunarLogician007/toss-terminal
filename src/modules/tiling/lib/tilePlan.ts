import type { Rect, TileRect } from "./layout";

/** One pane window TiledLayout draws. */
export type TilePlanItem = {
  key: string;
  id: number;
  rect: Rect;
  hidden: boolean;
  focused: boolean;
};

/**
 * What to draw for a layout. Panes are keyed by leaf id, so a reshape moves
 * a window rather than recreating its terminal.
 */
export function planTiles(
  next: TileRect[],
  activeLeafId: number,
): TilePlanItem[] {
  return next.map(({ id, x, y, width, height, hidden }) => ({
    key: `leaf-${id}`,
    id,
    rect: { x, y, width, height },
    hidden,
    focused: id === activeLeafId,
  }));
}
