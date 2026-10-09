import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { sharedApi as api, history, row } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: () => undefined };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import i18n from "./i18n";
import { useStore, defaultSelection } from "./store";
import Library from "./pages/Library";
import GameDetail from "./pages/GameDetail";
import { CollectionExportDialog, CollectionImportDialog } from "./pages/CollectionDialogs";
import type { BundlePreview, GameRow } from "./types";

function seed(rows: GameRow[]) {
  const map = Object.fromEntries(rows.map((r) => [r.id, r]));
  useStore.setState({ rows: map, order: rows.map((r) => r.id), selected: defaultSelection(rows), stateFilter: "all", platformFilter: "all", query: "", loading: false, toasts: [], undoable: {}, view: { name: "library" } });
}

function bundle(over: Partial<BundlePreview>): BundlePreview {
  return {
    gameId: "steam-367520",
    gameName: "Hollow Knight",
    localGame: true,
    createGame: false,
    createdAt: "2026-10-09T12:00:00+08:00",
    host: "OLD-PC",
    nodes: [],
    newSnapshots: 0,
    missing: [],
    refs: [],
    locationMismatch: false,
    ...over,
  };
}

beforeEach(async () => {
  vi.clearAllMocks();
  dialog.ask.mockResolvedValue(true);
  await i18n.changeLanguage("zh-CN");
});

describe("游戏库", () => {
  it("检查更新完成后提示结果，未安装的游戏有标记", async () => {
    const rows = [row(), row({ id: "steam-1", name: "Cuphead", nameZh: null, state: "up-to-date", installed: false })];
    seed(rows);
    api.loadLibrary.mockResolvedValue(rows);
    render(<Library />);
    expect(screen.getByText("未安装")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /检查更新/ }));
    await waitFor(() => expect(useStore.getState().toasts.map((t) => t.text)).toContain("检查完成：1 个游戏有更新"));
  });
});

describe("存档安全保护开关", () => {
  it("在游戏库中显示状态，关闭需确认并提示", async () => {
    const info = { version: "0.1.0", portable: false, firstRun: false, locale: "zh-CN", theme: "auto" as const, configPath: "", defaultRepoDir: "", repoDir: "", configError: null, host: "MY-PC", protectLocal: true };
    seed([row()]);
    useStore.setState({ info });
    const cfg = { general: { theme: "auto" as const, close_to_tray: true }, backup: { include_config_files: true, size_warning_gb: 2, disabled_platforms: [], protect_local: true }, metadata: { online: true, igdb_client_id: "", igdb_client_secret: "", steamgriddb_api_key: "" }, log: { level: "info", keep_days: 14 } };
    api.getSettings.mockResolvedValue(cfg);
    api.saveSettings.mockResolvedValue({ ...info, protectLocal: false });
    render(<Library />);
    const btn = screen.getByRole("button", { name: /存档安全保护已开启：本机（MY-PC）/ });
    expect(btn).toHaveTextContent("保护已开启");
    // 取消确认：不改动。
    dialog.ask.mockResolvedValueOnce(false);
    fireEvent.click(btn);
    await waitFor(() => expect(dialog.ask).toHaveBeenCalled());
    expect(api.saveSettings).not.toHaveBeenCalled();
    fireEvent.click(btn);
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backup: expect.objectContaining({ protect_local: false }) })));
    expect(await screen.findByText("保护已关闭")).toBeInTheDocument();
    expect(useStore.getState().toasts.map((t) => t.text)).toContain("已关闭存档安全保护：现在可以删除本机备份的快照和分支，删除后不能撤销");
    useStore.setState({ info: null });
  });
});

describe("导出多个游戏", () => {
  it("还没有快照的游戏不能选，导出所选为合集包", async () => {
    const rows = [row(), row({ id: "steam-1", name: "Cuphead", nameZh: null, snapshots: 0 })];
    dialog.save.mockResolvedValue("D:/out/all.bonfire");
    api.exportCollection.mockResolvedValue({ dest: "D:/out/all.bonfire", bytes: 4096, games: [{ gameId: "steam-367520", gameName: "Hollow Knight", dir: "games/0/", snapshots: 2 }], objects: 9, skipped: [] });
    render(<CollectionExportDialog rows={rows} initial={["steam-367520", "steam-1"]} onClose={() => {}} />);
    const cup = screen.getByText("Cuphead").closest("label")!;
    expect(within(cup).getByRole("checkbox")).toBeDisabled();
    expect(screen.getByText("已选 1 个游戏，共 2 个快照")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /选择位置并导出/ }));
    await waitFor(() => expect(api.exportCollection).toHaveBeenCalledWith(["steam-367520"], "D:/out/all.bonfire"));
    expect(await screen.findByRole("button", { name: /在资源管理器中显示/ })).toBeInTheDocument();
  });
});

describe("在游戏库中导入", () => {
  it("合集包：列出游戏与状态，导入所选，逐个显示结果", async () => {
    dialog.open.mockResolvedValue("D:/in/all.bonfire");
    api.inspectArchive.mockResolvedValue({
      collection: true,
      createdAt: "2026-10-09T12:00:00+08:00",
      host: "OLD-PC",
      games: [
        { key: "games/0/", preview: bundle({ newSnapshots: 2 }) },
        { key: "games/1/", preview: bundle({ gameId: "steam-9", gameName: "Hades", localGame: false, createGame: true, newSnapshots: 3 }) },
        { key: "games/2/", preview: bundle({ gameId: "steam-8", gameName: "Celeste" }) },
      ],
    });
    api.importArchive.mockResolvedValue([
      { key: "games/0/", gameId: "steam-367520", gameName: "Hollow Knight", report: { importedSnapshots: 2, objects: 5, createdGame: false, preview: bundle({}) }, error: null },
      { key: "games/1/", gameId: "steam-9", gameName: "Hades", report: { importedSnapshots: 3, objects: 7, createdGame: true, preview: bundle({}) }, error: null },
    ]);
    const onDone = vi.fn();
    render(<CollectionImportDialog onClose={() => {}} onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: /历史包 \/ 合集包/ }));
    expect(await screen.findByText("游戏库中没有，将加入（未安装）")).toBeInTheDocument();
    // 没有新内容的游戏不能勾选。
    expect(within(screen.getByText("Celeste").closest("label")!).getByRole("checkbox")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "导入所选（2）" }));
    await waitFor(() =>
      expect(api.importArchive).toHaveBeenCalledWith("D:/in/all.bonfire", [
        { key: "games/0/", target: null },
        { key: "games/1/", target: null },
      ]),
    );
    expect(await screen.findByText(/已加入游戏库/)).toBeInTheDocument();
    expect(onDone).toHaveBeenCalled();
  });

  it("存档文件：按说明找到游戏，没有时提示会加入游戏库", async () => {
    dialog.open.mockResolvedValue("D:/in/share.zip");
    api.inspectFilesSource.mockResolvedValue({
      manifest: { gameId: "steam-9", gameName: "Hades", snapshot: "b3:eeeeeeee", snapshotCreatedAt: null, comment: null, exportedAt: "2026-10-09T12:00:00+08:00", host: "FRIEND-PC" },
      gameId: null,
      preview: null,
    });
    api.importFilesAuto.mockResolvedValue({ gameId: "steam-9", gameName: "Hades", createdGame: true, report: {} });
    render(<CollectionImportDialog onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /zip 存档/ }));
    expect(await screen.findByText("这些存档属于「Hades」")).toBeInTheDocument();
    expect(screen.getByText(/游戏库中没有这个游戏/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^导入$/ }));
    await waitFor(() => expect(api.importFilesAuto).toHaveBeenCalledWith("D:/in/share.zip", null));
    expect(await screen.findByRole("button", { name: "查看" })).toBeInTheDocument();
  });
});

describe("删除", () => {
  it("删除快照前预览影响并确认；带标签的快照不能删", async () => {
    seed([row()]);
    api.history.mockResolvedValue(history());
    api.snapshotFiles.mockResolvedValue([]);
    api.diff.mockResolvedValue([]);
    api.gameRow.mockResolvedValue(row());
    api.deleteSnapshot.mockResolvedValue({ removed: ["b3:cccccccc"], rewritten: 0, branchesDeleted: [], headBranch: null, dryRun: true });
    render(<GameDetail id="steam-367520" />);
    await screen.findAllByText("白宫之前");
    const more = () => screen.getAllByRole("button", { name: "更多操作" }).at(-1)!;
    fireEvent.click(more());
    fireEvent.click(screen.getByRole("menuitem", { name: "删除这个快照…" }));
    await waitFor(() => expect(api.deleteSnapshot).toHaveBeenCalledWith("steam-367520", "b3:cccccccc", false));
    expect(api.deleteSnapshot).toHaveBeenCalledWith("steam-367520", "b3:cccccccc", true);
    expect(dialog.ask.mock.calls[0][0]).toContain("当前存档对应的快照");

    // 带标签“Boss 前”的快照：删除快照不可用，可以删除标签。
    fireEvent.click(screen.getByRole("option", { name: /打完螳螂领主/ }));
    await screen.findByText(/第二行/);
    fireEvent.click(more());
    expect(screen.getByRole("menuitem", { name: "删除这个快照…" })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "删除标签 Boss 前" }));
    await waitFor(() => expect(api.deleteTag).toHaveBeenCalledWith("steam-367520", "Boss 前"));
  });

  it("存档安全保护：本机备份的快照不能删除", async () => {
    seed([row()]);
    const node = history().nodes.find((n) => n.id === "b3:cccccccc")!;
    useStore.setState({ info: { version: "0.1.0", portable: false, firstRun: false, locale: "zh-CN", theme: "auto", configPath: "", defaultRepoDir: "", repoDir: "", configError: null, host: node.host, protectLocal: true } });
    api.history.mockResolvedValue(history());
    api.snapshotFiles.mockResolvedValue([]);
    api.diff.mockResolvedValue([]);
    render(<GameDetail id="steam-367520" />);
    await screen.findAllByText("白宫之前");
    expect(screen.getByText("本机备份")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "更多操作" }).at(-1)!);
    const item = screen.getByRole("menuitem", { name: "删除这个快照…" });
    expect(item).toBeDisabled();
    expect(item).toHaveAttribute("title", "存档安全保护：本机备份的快照不能删除（可在设置中关闭）");
    useStore.setState({ info: null });
  });

  it("删除分支时说明会删除多少个快照", async () => {
    seed([row()]);
    api.history.mockResolvedValue(history());
    api.snapshotFiles.mockResolvedValue([]);
    api.diff.mockResolvedValue([]);
    api.gameRow.mockResolvedValue(row());
    api.deleteBranch.mockResolvedValue({ removed: ["b3:cccccccc", "b3:bbbbbbbb"], rewritten: 0, branchesDeleted: ["main"], headBranch: null, dryRun: true });
    dialog.ask.mockResolvedValue(false);
    render(<GameDetail id="steam-367520" />);
    await screen.findAllByText("白宫之前");
    fireEvent.click(screen.getAllByRole("button", { name: "更多操作" }).at(-1)!);
    fireEvent.click(screen.getByRole("menuitem", { name: "删除分支 main…" }));
    await waitFor(() => expect(dialog.ask).toHaveBeenCalled());
    expect(dialog.ask.mock.calls[0][0]).toContain("2 个快照");
    // 取消：不删除。
    expect(api.deleteBranch).toHaveBeenCalledTimes(1);
  });
});
