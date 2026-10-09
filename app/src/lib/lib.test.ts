import { describe, expect, it } from "vitest";
import { bytes, displayPath, firstLine, hours, relative, shortId } from "./format";
import { layout } from "./timeline";

describe("format", () => {
  it("大小与 Rust 端一致", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(1023)).toBe("1023 B");
    expect(bytes(1024)).toBe("1.0 KB");
    expect(bytes(3_250_586)).toBe("3.1 MB");
  });

  it("相对时间", () => {
    const now = new Date("2026-10-09T12:00:00Z");
    expect(relative(new Date("2026-10-09T11:57:00Z"), "zh-CN", now)).toBe("3分钟前");
    expect(relative(new Date("2026-10-08T12:00:00Z"), "en", now)).toBe("yesterday");
  });

  it("其他小工具", () => {
    expect(hours(2520)).toBe("42.0");
    expect(shortId("b3:abcdef0123456789")).toBe("abcdef01");
    expect(displayPath("0/子目录/存档 ã.sav")).toBe("子目录/存档 ã.sav");
    expect(firstLine("第一行\n第二行")).toBe("第一行");
  });
});

describe("timeline layout", () => {
  it("一条直线在同一列", () => {
    const g = layout([
      { id: "c", parents: ["b"] },
      { id: "b", parents: ["a"] },
      { id: "a", parents: [] },
    ]);
    expect(g.rows.map((r) => r.col)).toEqual([0, 0, 0]);
    expect(g.columns).toBe(1);
    expect(g.edges).toEqual([
      { fromRow: 0, fromCol: 0, toRow: 1, toCol: 0 },
      { fromRow: 1, fromCol: 0, toRow: 2, toCol: 0 },
    ]);
  });

  it("恢复旧快照后的新分支分到另一列，并在分叉点汇合", () => {
    // main: a ← b ← c；从 a 恢复后继续玩：a ← x（新分支，时间最新）。
    const g = layout([
      { id: "x", parents: ["a"] },
      { id: "c", parents: ["b"] },
      { id: "b", parents: ["a"] },
      { id: "a", parents: [] },
    ]);
    const col = Object.fromEntries(g.rows.map((r) => [r.id, r.col]));
    expect(col.x).toBe(0);
    expect(col.c).toBe(1);
    expect(col.b).toBe(1);
    expect(col.a).toBe(0);
    expect(g.columns).toBe(2);
    expect(g.edges).toContainEqual({ fromRow: 2, fromCol: 1, toRow: 3, toCol: 0 });
  });

  it("父快照不在列表中（被清理）时不画线", () => {
    const g = layout([{ id: "b", parents: ["gone"] }]);
    expect(g.edges).toEqual([]);
  });

  it("空列表", () => {
    expect(layout([])).toEqual({ rows: [], edges: [], columns: 1 });
  });
});

describe("历史段", () => {
  it("祖先与一段历史", async () => {
    const { ancestors, rangeOf } = await import("./timeline");
    // a ← b ← c，b ← d（d 从 b 分叉）
    const nodes = [
      { id: "d", parents: ["b"] },
      { id: "c", parents: ["b"] },
      { id: "b", parents: ["a"] },
      { id: "a", parents: [] },
    ];
    expect([...ancestors(nodes, ["c"])].sort()).toEqual(["a", "b", "c"]);
    expect([...ancestors(nodes, ["c", "d"])].sort()).toEqual(["a", "b", "c", "d"]);
    expect([...rangeOf(nodes, "b", "c")].sort()).toEqual(["b", "c"]);
    expect([...rangeOf(nodes, null, "d")].sort()).toEqual(["a", "b", "d"]);
  });
});
