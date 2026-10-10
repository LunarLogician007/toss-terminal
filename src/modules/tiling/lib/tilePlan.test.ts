import { describe, expect, it } from "vitest";
import { planTiles } from "./tilePlan";

const t = (id: number, x: number, w: number) => ({
  id,
  x,
  y: 0,
  width: w,
  height: 100,
  hidden: false,
});

describe("planTiles", () => {
  it("keys every tile by leaf id so a reshape never recreates a terminal", () => {
    const plan = planTiles([t(1, 0, 97), t(2, 103, 97)], 2);
    expect(plan.map((p) => p.key)).toEqual(["leaf-1", "leaf-2"]);
  });

  it("places each tile at its rect", () => {
    expect(planTiles([t(1, 0, 200)], 1)[0].rect).toEqual({
      x: 0,
      y: 0,
      width: 200,
      height: 100,
    });
  });

  it("marks the focused tile", () => {
    const plan = planTiles([t(1, 0, 97), t(2, 103, 97)], 2);
    expect(plan.map((p) => p.focused)).toEqual([false, true]);
  });
});
