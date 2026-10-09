import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { sharedApi as api, history, row } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: (p?: string | null) => (p ? `asset://${p}` : undefined) };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import i18n from "./i18n";
import { useStore, defaultSelection } from "./store";
import { groupByDay, shotCounts, shotSnapshots } from "./lib/shots";
import ShotsPanel from "./pages/ShotsPanel";
import GameDetail from "./pages/GameDetail";
import type { AlbumView, ShotRecord } from "./types";

function shot(blob: string, takenAt: string, over: Partial<ShotRecord> = {}): ShotRecord {
  return { gameId: "steam-367520", blob, takenAt, source: "steam", originalPath: `C:/Steam/shots/${blob}.jpg`, size: 2048, width: 1920, height: 1080, addedAt: takenAt, ...over };
}

function album(shots: ShotRecord[], over: Partial<AlbumView> = {}): AlbumView {
  return { shots, enabled: true, followsGlobal: true, globalEnabled: true, dirs: [], pending: 0, bytes: shots.reduce((n, s) => n + s.size, 0), sources: ["steam"], ...over };
}

beforeEach(async () => {
  vi.clearAllMocks();
  dialog.ask.mockResolvedValue(true);
  useStore.setState({ toasts: [] });
  await i18n.changeLanguage("zh-CN");
});

describe("截图与快照的对应", () => {
  const nodes = history().nodes; // a: 10-07, b: 10-08, c: 10-09（10:00 +08:00）
  it("归到拍摄之后最早的快照，之后没有快照的为空", () => {
    const shots = [shot("b3:s1", "2026-10-07T09:00:00+08:00"), shot("b3:s2", "2026-10-08T11:00:00+08:00"), shot("b3:s3", "2026-10-09T10:00:00+08:00"), shot("b3:s4", "2026-10-10T00:00:00+08:00")];
    const m = shotSnapshots(nodes, shots);
    expect(m.get("b3:s1")).toBe("b3:aaaaaaaa");
    expect(m.get("b3:s2")).toBe("b3:cccccccc");
    expect(m.get("b3:s3")).toBe("b3:cccccccc");
    expect(m.get("b3:s4")).toBeNull();
    expect(shotCounts(m).get("b3:cccccccc")).toBe(2);
  });

  it("按日期分组，新的在前", () => {
    // 同一天的两个时间选在 UTC 正午前后，任何时区下都落在同一天。
    const g = groupByDay([shot("b3:s1", "2026-10-07T12:00:00Z"), shot("b3:s2", "2026-10-09T12:30:00Z"), shot("b3:s3", "2026-10-09T11:30:00Z")]);
    expect(g.map((x) => x.shots.map((s) => s.blob))).toEqual([["b3:s2", "b3:s3"], ["b3:s1"]]);
  });
});

describe("截图标签页", () => {
  it("收集、多选导出与删除，单独关闭这个游戏的截图", async () => {
    const shots = [shot("b3:s1", "2026-10-07T09:00:00+08:00"), shot("b3:s2", "2026-10-09T11:00:00+08:00")];
    api.album.mockResolvedValue(album(shots, { pending: 3 }));
    api.syncShots.mockResolvedValue([{ gameId: "steam-367520", added: 3, bytes: 100, disabled: false }]);
    api.deleteShots.mockResolvedValue(1);
    api.exportShots.mockResolvedValue({ dest: "D:/out/s.zip", files: 1, bytes: 2048 });
    dialog.save.mockResolvedValue("D:/out/s.zip");
    const onChanged = vi.fn();
    render(<ShotsPanel gameId="steam-367520" gameName="空洞骑士" nodes={history().nodes} onChanged={onChanged} onShowSnapshot={() => {}} />);
    expect(await screen.findByText("磁盘上有 3 张新截图还没有收集")).toBeInTheDocument();
    expect(screen.getByText("Steam 截图")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /收集新截图/ }));
    await waitFor(() => expect(api.syncShots).toHaveBeenCalledWith("steam-367520"));
    await waitFor(() => expect(useStore.getState().toasts.map((t) => t.text)).toContain("收集了 3 张新截图"));

    fireEvent.click(screen.getByRole("checkbox", { name: "b3:s2.jpg" }));
    expect(screen.getByText("已选 1 张")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /导出/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "导出为 zip…" }));
    await waitFor(() => expect(api.exportShots).toHaveBeenCalledWith("steam-367520", ["b3:s2"], "D:/out/s.zip", "zip"));

    fireEvent.click(screen.getByRole("button", { name: /删除/ }));
    await waitFor(() => expect(api.deleteShots).toHaveBeenCalledWith("steam-367520", ["b3:s2"]));
    expect(dialog.ask.mock.calls[0][0]).toContain("磁盘上的原图不受影响");

    fireEvent.change(screen.getByRole("combobox", { name: "备份截图" }), { target: { value: "off" } });
    await waitFor(() => expect(api.setShotsEnabled).toHaveBeenCalledWith("steam-367520", false));
  });

  it("点开大图，显示之后的存档并可跳到分支图", async () => {
    api.album.mockResolvedValue(album([shot("b3:s1", "2026-10-08T11:00:00+08:00")]));
    const onShow = vi.fn();
    render(<ShotsPanel gameId="steam-367520" gameName="空洞骑士" nodes={history().nodes} onChanged={() => {}} onShowSnapshot={onShow} />);
    fireEvent.click(await screen.findByTitle("b3:s1.jpg"));
    const box = await screen.findByRole("dialog");
    await waitFor(() => expect(within(box).getByRole("img")).toHaveAttribute("src", "asset://C:/cache/full.png"));
    expect(within(box).getByText("1920 × 1080 · 2.0 KB")).toBeInTheDocument();
    fireEvent.click(within(box).getByRole("button", { name: /白宫之前/ }));
    expect(onShow).toHaveBeenCalledWith("b3:cccccccc");
  });

  it("没有截图时说明怎么截图，并可添加截图文件夹", async () => {
    api.album.mockResolvedValue(album([], { sources: [] }));
    dialog.open.mockResolvedValue("D:/MyShots");
    render(<ShotsPanel gameId="steam-367520" gameName="空洞骑士" nodes={[]} onChanged={() => {}} onShowSnapshot={() => {}} />);
    expect(await screen.findByText("还没有截图")).toBeInTheDocument();
    expect(screen.getByText(/没有找到这个游戏的截图来源/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /添加文件夹/ }));
    await waitFor(() => expect(api.addShotDir).toHaveBeenCalledWith("steam-367520", "D:/MyShots"));
  });
});

describe("详情页", () => {
  it("时间线标出快照前拍的截图，快照详情列出这些截图", async () => {
    const rows = [row({ shots: 2, newShots: 1 })];
    useStore.setState({ rows: Object.fromEntries(rows.map((r) => [r.id, r])), order: rows.map((r) => r.id), selected: defaultSelection(rows), toasts: [], undoable: {}, info: null });
    api.history.mockResolvedValue(history());
    api.snapshotFiles.mockResolvedValue([]);
    api.diff.mockResolvedValue([]);
    api.album.mockResolvedValue(album([shot("b3:s1", "2026-10-08T11:00:00+08:00"), shot("b3:s2", "2026-10-09T09:00:00+08:00")]));
    render(<GameDetail id="steam-367520" />);
    expect(await screen.findByText("这次存档前拍的截图（2）")).toBeInTheDocument();
    expect(screen.getByTitle("2 张截图")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "查看全部" }));
    expect(await screen.findByRole("heading", { name: "截图相册" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /截图/ })).toHaveAttribute("aria-selected", "true");
  });
});
