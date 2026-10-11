import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { row, sharedApi as api } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: () => undefined };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("./lib/confirm", () => ({ confirm: (...a: unknown[]) => dialog.ask(...a), ConfirmHost: () => null }));

import i18n from "./i18n";
import { resetUiStateForTest } from "./lib/uiState";
import { useStore } from "./store";
import OverviewPage from "./pages/OverviewPage";
import type { Activity, Attention, StorageStats, TargetStatus } from "./types";

const now = new Date().toISOString();
const failure: Attention = {
  items: [{ key: "sync:t1", kind: "sync", name: "NAS", game: null, target: "t1", issue: "network", error: "联网失败", at: now, retry: { attempt: 1, at: now } }],
  snoozed: 0,
};
const nas: TargetStatus = {
  target: { id: "t1", name: "NAS", kind: "folder", place: "network", url: "\\\\nas\\b", branch: "main", user: "", content: "repo", shots: true, settings: true, games: [], max_file_mb: 0, max_total_mb: 0, schedule: "after-backup", at: "21:00", every_hours: 6, enabled: true, ignore: [] },
  hasSecret: false,
  last: { at: now, uploaded: 0, uploadedBytes: 0, deleted: 0, excluded: [], error: "联网失败", issue: "network" },
  remoteFiles: 0,
  retry: { attempt: 1, at: now },
};
const stats: StorageStats = {
  repoDir: "D:\\Bonfire\\repo",
  total: 5_000_000,
  saves: 3_000_000,
  shots: 1_500_000,
  other: 500_000,
  free: 99_000_000_000,
  logical: 30_000_000,
  snapshots: 12,
  games: [{ id: "a", name: "Celeste", snapshots: 12, saves: 3_000_000, shots: 1_500_000 }],
  trend: [["2026-10-10", 4_000_000], ["2026-10-11", 5_000_000]],
};
const act = (over: Partial<Activity>): Activity => ({ at: now, action: "backup", ok: true, ...over });

beforeEach(async () => {
  vi.clearAllMocks();
  resetUiStateForTest();
  const rows = [
    row({ id: "a", name: "Celeste", nameZh: null, state: "up-to-date", snapshots: 12, lastBackupAt: now }),
    row({ id: "b", name: "Hades", nameZh: null, state: "has-changes", snapshots: 3, lastBackupAt: now }),
  ];
  useStore.setState({ toasts: [], attention: failure, order: rows.map((r) => r.id), rows: Object.fromEntries(rows.map((r) => [r.id, r])) });
  api.attention.mockResolvedValue(failure);
  api.syncStatus.mockResolvedValue([nas]);
  api.storageStats.mockResolvedValue(stats);
  api.cleanHistory.mockResolvedValue({ games: [], removed: 0, gc: { chunks: 0, blobs: 0, trees: 0, snapshots: [], bytes: 300 * 1024 * 1024, tmpFiles: 0, dryRun: true }, dryRun: true });
  api.compareDisk.mockResolvedValue({ at: now, compared: 2, games: [{ id: "a", name: "Celeste", new: 2, duplicates: 1, missing: 3, restorable: 2, toHome: 1, noPlace: 0, ignored: 0, error: null }] });
  api.activityList.mockResolvedValue({
    items: [act({ action: "sync", ok: false, name: "NAS", issue: "network", error: "联网失败" }), act({ action: "backup-auto", game: "a", name: "Celeste", n: 2, bytes: 1200, detail: "auto-change" })],
    more: false,
    failed: 1,
  });
  await i18n.changeLanguage("zh-CN");
});

describe("总览", () => {
  it("三道防线与待处理", async () => {
    render(<OverviewPage />);
    expect(await screen.findByText("1 个游戏的存档在各处都有，还有 1 个游戏需要处理")).toBeInTheDocument();
    expect(screen.getByText("1 个未备份")).toBeInTheDocument();
    expect(screen.getByText("1 个目标失败")).toBeInTheDocument();
    const todo = screen.getByRole("region", { name: "待处理" });
    expect(within(todo).getByText("「NAS」同步失败：连不上目标")).toBeInTheDocument();
    expect(within(todo).getByText("1 个游戏有新存档还没备份")).toBeInTheDocument();
    expect(await within(todo).findByText("清理历史可以释放 300.0 MB")).toBeInTheDocument();
    // 失败类的“稍后提醒”记在后台；提醒类记在界面状态中。
    fireEvent.click(within(todo).getAllByRole("button", { name: "稍后提醒" })[0]);
    await waitFor(() => expect(api.snoozeAttention).toHaveBeenCalledWith("sync:t1", now));
    fireEvent.click(within(todo).getAllByRole("button", { name: "稍后提醒" })[1]);
    expect(within(todo).queryByText("1 个游戏有新存档还没备份")).toBeNull();
    expect(screen.getByText(/条已设为稍后提醒/)).toBeInTheDocument();
  });

  it("存储与截图：一键收集、一键写回", async () => {
    api.collectAllShots.mockResolvedValue(2);
    api.writeBackAll.mockResolvedValue({ written: 3, bytes: 3000, exists: 0, noDir: 0, dirs: [] });
    render(<OverviewPage />);
    const storage = screen.getByRole("region", { name: "存储" });
    expect(await within(storage).findByText("存档快照 2.9 MB")).toBeInTheDocument();
    expect(within(storage).getByText("Celeste")).toBeInTheDocument();
    const disk = screen.getByRole("region", { name: "截图与磁盘" });
    expect(await within(disk).findByText(/2 张写回原位置/)).toBeInTheDocument();
    fireEvent.click(within(disk).getByRole("button", { name: "全部收集" }));
    await waitFor(() => expect(api.collectAllShots).toHaveBeenCalled());
    fireEvent.click(within(disk).getByRole("button", { name: "全部写回磁盘" }));
    await waitFor(() => expect(api.writeBackAll).toHaveBeenCalled());
    expect(dialog.ask).toHaveBeenCalledWith(expect.stringContaining("把 3 张截图写到磁盘"), expect.anything());
  });

  it("操作记录：按天分组、筛选失败的", async () => {
    render(<OverviewPage />);
    const log = screen.getByRole("region", { name: "操作记录" });
    expect(await within(log).findByText("今天")).toBeInTheDocument();
    expect(within(log).getByText("同步到 NAS失败：连不上目标")).toBeInTheDocument();
    expect(within(log).getByText(/存档变化后、2 个文件/)).toBeInTheDocument();
    fireEvent.click(within(log).getByRole("button", { name: /失败/ }));
    await waitFor(() => expect(api.activityList).toHaveBeenLastCalledWith(expect.objectContaining({ failed: true, category: null })));
    fireEvent.click(within(log).getByRole("button", { name: "备份" }));
    await waitFor(() => expect(api.activityList).toHaveBeenLastCalledWith(expect.objectContaining({ failed: false, category: "backup" })));
  });
});
