import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { sharedApi as api, row } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: () => undefined };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("./lib/confirm", () => ({ confirm: (...a: unknown[]) => dialog.ask(...a), ConfirmHost: () => null }));

import i18n from "./i18n";
import { useStore, defaultSelection, initialView, readFilters } from "./store";
import Library from "./pages/Library";
import Gallery from "./pages/Gallery";
import Settings from "./pages/Settings";
import ScanPage from "./pages/ScanPage";
import ShotDiffDialog from "./pages/ShotDiffDialog";
import CloseDialog from "./components/CloseDialog";
import { ProcessDialog } from "./components/ProcessDialog";
import { DeleteSnapshotsDialog } from "./components/DeleteSnapshotsDialog";
import { noteGames } from "./lib/fresh";
import { loadUiState, resetUiStateForTest } from "./lib/uiState";
import type { Config, GalleryView, GameRow, ShotRecord } from "./types";

const info = { version: "0.1.0", portable: false, firstRun: false, locale: "zh-CN", theme: "auto" as const, configPath: "C:/c.toml", defaultRepoDir: "C:/repo", repoDir: "C:/repo", configError: null, host: "PC", protectLocal: true };

const cfg = {
  general: { theme: "auto", close_to_tray: true, close_asked: false },
  backup: { include_config_files: true, size_warning_gb: 2, disabled_platforms: [], protect_local: true },
  shots: { enabled: true, schedule: "off", at: "21:00", weekday: 7, every_hours: 6 },
  auto_backup: { schedule: "off", at: "21:00", weekday: 7, every_hours: 6, on_change: false, check_minutes: 5, quiet_minutes: 2 },
  metadata: { online: true, igdb_client_id: "", igdb_client_secret: "", steamgriddb_api_key: "" },
  log: { level: "info", keep_days: 14 },
} as unknown as Config;

function seed(rows: GameRow[]) {
  useStore.setState({
    info,
    rows: Object.fromEntries(rows.map((r) => [r.id, r])),
    order: rows.map((r) => r.id),
    selected: defaultSelection(rows),
    stateFilter: "all",
    modeFilter: "all",
    profileFilter: null,
    newOnly: false,
    fresh: {},
    platformFilter: "all",
    query: "",
    loading: false,
    toasts: [],
    undoable: {},
    view: { name: "library" },
  });
}

function shot(gameId: string, blob: string, takenAt: string, over: Partial<ShotRecord> = {}): ShotRecord {
  return { gameId, blob, takenAt, source: "steam", originalPath: `C:/Steam/${blob}.jpg`, size: 1000, addedAt: takenAt, file: `${blob}.jpg`, ...over };
}

beforeEach(async () => {
  vi.clearAllMocks();
  resetUiStateForTest();
  dialog.ask.mockResolvedValue(true);
  api.getSettings.mockResolvedValue(structuredClone(cfg));
  await i18n.changeLanguage("zh-CN");
});

describe("游戏库：管理方式", () => {
  const rows = () => [
    row({ id: "a", name: "Alpha", nameZh: null, mode: "both" }),
    row({ id: "b", name: "Beta", nameZh: null, mode: "saves", state: "up-to-date" }),
    row({ id: "c", name: "Gamma", nameZh: null, mode: "shots", shotsOnly: true, state: "shots-only" }),
  ];

  it("按管理方式筛选，数量正确", () => {
    seed(rows());
    render(<Library />);
    // 管理方式单独一组按钮，不在“筛选”菜单中。
    const group = screen.getByRole("group", { name: "按管理方式筛选" });
    expect(within(group).getByRole("button", { name: "全部（3）" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(within(group).getByRole("button", { name: "只管理截图（1）" }));
    expect(screen.getByText("Gamma")).toBeInTheDocument();
    expect(screen.queryByText("Alpha")).toBeNull();
    expect(within(group).getByRole("button", { name: "只管理截图（1）" })).toHaveAttribute("aria-pressed", "true");
    expect(within(group).getByRole("button", { name: "只管理截图（1）" })).toHaveClass("mode-shots");
    fireEvent.click(within(group).getByRole("button", { name: "只管理存档（1）" }));
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.queryByText("Gamma")).toBeNull();
    expect(screen.getByRole("button", { name: "按筛选方案、状态与平台筛选" })).toHaveTextContent("筛选");
  });

  it("每行的管理方式标签可以直接切换，批量切换所选游戏", async () => {
    seed(rows());
    api.gameRow.mockImplementation(async (id: string) => rows().find((r) => r.id === id)!);
    render(<Library />);
    fireEvent.click(screen.getByRole("button", { name: /管理方式，点击切换：Beta/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /存档和截图/ }));
    await waitFor(() => expect(api.setManageMode).toHaveBeenCalledWith(["b"], "both"));
    await waitFor(() => expect(useStore.getState().toasts.map((t) => t.text)).toContain("「Beta」改为存档和截图"));

    // 默认勾选了 Alpha 与 Gamma（不是“已是最新”）。
    fireEvent.click(screen.getByRole("button", { name: /所选游戏/ }));
    // 批量切换先确认；取消时不改动。
    dialog.ask.mockResolvedValueOnce(false);
    fireEvent.click(screen.getByRole("menuitem", { name: /只管理存档/ }));
    await waitFor(() => expect(dialog.ask).toHaveBeenCalled());
    const [text, opts] = dialog.ask.mock.calls[0];
    expect(text).toContain("把所选的 2 个游戏改为“只管理存档”？");
    expect(text).toContain("磁盘上的存档都不受影响");
    expect(opts.title).toBe("批量切换管理方式");
    expect(api.setManageMode).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /所选游戏/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: /只管理截图/ }));
    await waitFor(() => expect(api.setManageMode).toHaveBeenCalledWith(["a", "c"], "shots"));
    expect(dialog.ask.mock.calls[1][0]).toContain("其中 1 个已经是这种方式");
  });

  it("自动备份快捷开关：显示状态并一键设置", async () => {
    seed(rows());
    api.saveSettings.mockResolvedValue(info);
    render(<Library />);
    const btn = await screen.findByRole("button", { name: /自动备份：快速设置/ });
    expect(btn).toHaveTextContent("自动备份：关");
    fireEvent.click(btn);
    fireEvent.click(screen.getByRole("menuitemradio", { name: "每天 21:00" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalled());
    expect(api.saveSettings.mock.calls[0][0].auto_backup.schedule).toBe("daily");
    await waitFor(() => expect(screen.getByRole("button", { name: /自动备份：快速设置/ })).toHaveTextContent("自动备份：每天 21:00"));

    fireEvent.click(screen.getByRole("button", { name: /自动备份：快速设置/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /存档变化后自动备份/ }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledTimes(2));
    expect(api.saveSettings.mock.calls[1][0].auto_backup).toMatchObject({ schedule: "daily", on_change: true });
    // 检查间隔：开启变化后自动备份后可选，按钮上显示。
    await waitFor(() => expect(screen.getByRole("button", { name: /自动备份：快速设置/ })).toHaveTextContent("变化后（每 5 分钟检查）"));
    fireEvent.click(screen.getByRole("button", { name: /自动备份：快速设置/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "每 15 分钟检查一次" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledTimes(3));
    expect(api.saveSettings.mock.calls[2][0].auto_backup).toMatchObject({ on_change: true, check_minutes: 15 });
  });
});

describe("筛选方案", () => {
  it("在“筛选”菜单中选择方案：只列出并勾选方案中的游戏，可取消（恢复原来的勾选）与删除方案", async () => {
    const rows = [row({ id: "a", name: "Alpha", nameZh: null }), row({ id: "b", name: "Beta", nameZh: null, state: "up-to-date" }), row({ id: "c", name: "Gamma", nameZh: null, state: "up-to-date" })];
    seed(rows);
    api.profiles.mockResolvedValue([{ name: "周末", gameIds: ["b", "c"], defaultComment: "" }]);
    api.deleteProfile.mockResolvedValue(undefined);
    render(<Library />);
    // 默认勾选了 Alpha（有更新）。
    const btn = () => screen.getByRole("button", { name: "按筛选方案、状态与平台筛选" });
    await waitFor(() => expect(api.profiles).toHaveBeenCalled());
    fireEvent.click(btn());
    fireEvent.click(await screen.findByRole("menuitemradio", { name: /周末/ }));
    expect(btn()).toHaveTextContent("筛选：方案 周末");
    expect(useStore.getState().selected).toEqual(new Set(["b", "c"]));
    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.getByText("Beta")).toBeInTheDocument();
    fireEvent.click(btn());
    fireEvent.click(screen.getByRole("menuitem", { name: "不使用筛选方案" }));
    expect(useStore.getState().selected).toEqual(new Set(["a"]));
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(btn()).toHaveTextContent(/^筛选$/);

    fireEvent.click(btn());
    fireEvent.click(screen.getByRole("menuitemradio", { name: /周末/ }));
    fireEvent.click(btn());
    api.profiles.mockResolvedValue([]);
    fireEvent.click(screen.getByRole("menuitem", { name: "删除筛选方案「周末」" }));
    await waitFor(() => expect(api.deleteProfile).toHaveBeenCalledWith("周末"));
    expect(dialog.ask.mock.calls.at(-1)?.[0]).toContain("只删除方案，不影响游戏和备份");
  });
});

describe("移出游戏库", () => {
  const rows = () => [row({ id: "a", name: "Alpha", nameZh: null, snapshots: 3, shots: 2 }), row({ id: "b", name: "Beta", nameZh: null, snapshots: 1, shots: 0 })];

  it("默认不删除备份；保护开启时不能勾选删除存档", async () => {
    seed(rows());
    api.removeGame.mockResolvedValue({ game: { id: "a" }, snapshots: 0, shots: 0 });
    render(<Library />);
    fireEvent.click(screen.getByRole("button", { name: /所选游戏/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "移出游戏库（2）" }));
    const dlg = screen.getByRole("dialog", { name: "移出 2 个游戏" });
    expect(within(dlg).getByText(/磁盘上的游戏存档和截图原图不会被删除/)).toBeInTheDocument();
    const snaps = within(dlg).getByRole("checkbox", { name: /同时删除备份的存档（共 4 个快照）/ });
    expect(snaps).not.toBeChecked();
    expect(snaps).toBeDisabled();
    expect(within(dlg).getByRole("checkbox", { name: /同时删除备份的截图（共 2 张/ })).not.toBeChecked();
    fireEvent.click(within(dlg).getByRole("button", { name: "移出游戏库" }));
    await waitFor(() => expect(api.removeGame).toHaveBeenCalledTimes(2));
    expect(api.removeGame).toHaveBeenCalledWith("a", { snapshots: false, shots: false });
    await waitFor(() => expect(useStore.getState().toasts.map((t) => t.text)).toContain("已移出 2 个游戏"));
  });

  it("保护关闭时可以同时删除备份的存档与截图", async () => {
    seed(rows());
    useStore.setState({ info: { ...info, protectLocal: false } });
    api.removeGame.mockResolvedValue({ game: { id: "a" }, snapshots: 3, shots: 2 });
    render(<Library />);
    fireEvent.click(screen.getByRole("button", { name: /所选游戏/ }));
    fireEvent.click(screen.getByRole("menuitem", { name: "移出游戏库（2）" }));
    const dlg = screen.getByRole("dialog", { name: "移出 2 个游戏" });
    fireEvent.click(within(dlg).getByRole("checkbox", { name: /同时删除备份的存档/ }));
    fireEvent.click(within(dlg).getByRole("checkbox", { name: /同时删除备份的截图/ }));
    expect(within(dlg).getAllByText("删除后不能恢复。")).toHaveLength(2);
    fireEvent.click(within(dlg).getByRole("button", { name: "移出并删除备份" }));
    await waitFor(() => expect(api.removeGame).toHaveBeenCalledWith("b", { snapshots: true, shots: true }));
  });
});

describe("图库", () => {
  const view = (): GalleryView => ({
    games: [
      { id: "g1", name: "Hollow Knight", nameZh: "空洞骑士", platform: "steam", inLibrary: true, mode: "both" },
      { id: "g2", name: "Celeste", nameZh: null, platform: "steam", inLibrary: false, mode: "shots" },
    ],
    shots: [
      shot("g1", "b3:111111", "2026-10-09T21:00:00+08:00"),
      shot("g2", "b3:222222", "2026-09-01T10:00:00+08:00", { source: "capture" }),
      shot("g1", "b3:333333", "2026-08-01T10:00:00+08:00"),
    ],
    trash: [shot("g2", "b3:444444", "2026-07-01T10:00:00+08:00", { deletedAt: "2026-10-01T10:00:00+08:00" })],
    bytes: 3000,
    trashBytes: 1000,
    dir: "D:/Repo/screenshots",
  });

  it("按游戏分组，可改为按月份并记住；按来源筛选", async () => {
    api.gallery.mockResolvedValue(view());
    render(<Gallery />);
    expect(await screen.findByText("3 张截图，2.9 KB，来自 2 个游戏")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "选择「空洞骑士」中的全部截图" })).toBeInTheDocument();
    // 每组都有全选与反选（U-49）：反选只作用于这一组。
    const hk = screen.getByRole("group", { name: "「空洞骑士」的选择" });
    fireEvent.click(within(hk).getByRole("button", { name: "反选" }));
    expect(screen.getByText("· 已选 2", { exact: false, selector: ".day-picked" })).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "选择「Celeste」中的全部截图" })).not.toBeChecked();
    fireEvent.click(within(hk).getByRole("checkbox", { name: "选择「空洞骑士」中的全部截图" }));
    expect(document.querySelector(".day-picked")).toBeNull();
    expect(screen.getAllByText("已移出游戏库").length).toBeGreaterThan(0);
    // 分组、排序与筛选都在一个“整理”菜单中，工具栏只占一行。
    const arrange = () => screen.getByRole("button", { name: "分组、排序与按来源 / 游戏筛选" });
    fireEvent.click(arrange());
    fireEvent.click(screen.getByRole("menuitemradio", { name: "按月份" }));
    expect(screen.getByRole("checkbox", { name: /2026年10月/ })).toBeInTheDocument();
    expect(localStorage.getItem("bonfire.gallery")).toContain("month");
    fireEvent.click(arrange());
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Windows 游戏栏截图" }));
    expect(screen.getAllByRole("checkbox", { name: /^b3|\.jpg$/ })).toHaveLength(1);
    expect(arrange()).toHaveTextContent("筛选：Windows 游戏栏截图");
    localStorage.removeItem("bonfire.gallery");
  });

  it("跨游戏删除按游戏分批，回收站可恢复与清空", async () => {
    api.gallery.mockResolvedValue(view());
    api.deleteShots.mockResolvedValue(1);
    api.purgeShots.mockResolvedValue({ purged: 1, originalsDeleted: 0, originalsKept: 0, originalsFailed: 0 });
    render(<Gallery />);
    await screen.findByText(/3 张截图/);
    fireEvent.click(screen.getByRole("checkbox", { name: "全选" }));
    fireEvent.click(screen.getByRole("button", { name: /删除/ }));
    await waitFor(() => expect(api.deleteShots).toHaveBeenCalledTimes(2));
    expect(api.deleteShots).toHaveBeenCalledWith("g1", ["b3:111111", "b3:333333"]);
    expect(api.deleteShots).toHaveBeenCalledWith("g2", ["b3:222222"]);

    fireEvent.click(screen.getByRole("tab", { name: /回收站/ }));
    expect(screen.getByText(/不会被再次收集/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "清空回收站" }));
    const confirm = screen.getByRole("dialog", { name: "彻底删除截图" });
    expect(within(confirm).getByText(/清空所有游戏的回收站（1 张截图）/)).toBeInTheDocument();
    fireEvent.click(within(confirm).getByRole("button", { name: "彻底删除" }));
    await waitFor(() => expect(api.purgeShots).toHaveBeenCalledWith("g2", ["b3:444444"], false));
  });
});

describe("图库：查看方式与框选", () => {
  it("切换为列表查看并记住；按住鼠标拖动框选，按住 Ctrl 加选", async () => {
    api.gallery.mockResolvedValue({
      games: [{ id: "g1", name: "Hollow Knight", nameZh: null, platform: "steam", inLibrary: true, mode: "both" }],
      shots: [shot("g1", "b3:111111", "2026-10-09T21:00:00+08:00"), shot("g1", "b3:222222", "2026-10-08T21:00:00+08:00"), shot("g1", "b3:333333", "2026-10-07T21:00:00+08:00")],
      trash: [],
      bytes: 3000,
      trashBytes: 0,
      dir: "D:/Repo/screenshots",
    });
    const { container } = render(<Gallery />);
    await screen.findByText(/3 张截图/);
    fireEvent.click(screen.getByRole("radio", { name: "列表" }));
    expect(container.querySelector(".shot-list")).not.toBeNull();
    expect(localStorage.getItem("bonfire.gallery")).toContain("list");
    fireEvent.click(screen.getByRole("radio", { name: "大图" }));
    expect(container.querySelector(".shot-grid.look-large")).not.toBeNull();

    // jsdom 没有布局：给每张截图一个位置（纵向排列，每张高 100）。
    const scroller = container.querySelector(".gallery") as HTMLElement;
    scroller.getBoundingClientRect = () => ({ left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0, toJSON: () => ({}) });
    Object.defineProperty(scroller, "clientWidth", { value: 780 });
    container.querySelectorAll<HTMLElement>("[data-key]").forEach((el, i) => {
      el.getBoundingClientRect = () => ({ left: 10, top: 10 + i * 100, right: 200, bottom: 100 + i * 100, width: 190, height: 90, x: 10, y: 10 + i * 100, toJSON: () => ({}) });
    });
    fireEvent.mouseDown(scroller, { button: 0, clientX: 5, clientY: 5 });
    fireEvent.mouseMove(window, { clientX: 300, clientY: 150 });
    fireEvent.mouseUp(window);
    // 底部统计跟随选择变化。
    expect(screen.getByText(/^已选 2 张，.*来自 1 个游戏$/)).toBeInTheDocument();
    // 按住 Ctrl 再框选第三张：加到已有的选择上。
    fireEvent.mouseDown(scroller, { button: 0, clientX: 5, clientY: 230, ctrlKey: true });
    fireEvent.mouseMove(window, { clientX: 300, clientY: 260 });
    fireEvent.mouseUp(window);
    expect(screen.getByText(/^已选 3 张，/)).toBeInTheDocument();
    localStorage.removeItem("bonfire.gallery");
  });
});

describe("截图与磁盘对比", () => {
  it("显示新截图、被忽略的和缺失的，可写回原位置与重新收集", async () => {
    api.shotDiff.mockResolvedValue({
      gameId: "g1",
      new: [{ path: "C:/Steam/new.jpg", size: 1, source: "steam" }],
      duplicates: 1,
      ignored: [{ path: "C:/Steam/old.jpg", size: 1, blob: "b3:555555", purged: true }],
      missing: [shot("g1", "b3:666666", "2026-10-01T10:00:00+08:00")],
      restorable: 1,
      synced: 4,
      home: "C:/Steam/shots",
    });
    api.syncShotsToDisk.mockResolvedValue({ written: 1, bytes: 1000, exists: 0, noDir: 0, dirs: ["C:/Steam"] });
    api.recollectShots.mockResolvedValue(1);
    const changed = vi.fn();
    render(<ShotDiffDialog gameId="g1" gameName="空洞骑士" onClose={() => {}} onChanged={changed} />);
    expect(await screen.findByText(/另有 1 张与已收集的截图内容相同/)).toBeInTheDocument();
    expect(screen.getByText(/0 张在回收站中、1 张已彻底删除/)).toBeInTheDocument();
    expect(screen.getByText("有 3 张截图两边不一致")).toBeInTheDocument();
    expect(screen.getByText(/1 张的原文件夹还在，会写回原位置/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "写回磁盘（1）" }));
    await waitFor(() => expect(api.syncShotsToDisk).toHaveBeenCalledWith("g1", ["b3:666666"]));
    expect(dialog.ask.mock.calls[0][0]).toContain("不覆盖");
    fireEvent.click(screen.getByRole("button", { name: "重新收集 1 张" }));
    await waitFor(() => expect(api.recollectShots).toHaveBeenCalledWith("g1", []));
    expect(changed).toHaveBeenCalled();
  });

  it("从其他电脑导入的截图：原文件夹不在本机时写到本机截图文件夹；一致时显示已同步", async () => {
    api.shotDiff.mockResolvedValueOnce({
      gameId: "g1",
      new: [],
      duplicates: 0,
      ignored: [],
      missing: [shot("g1", "b3:777777", "2026-10-01T10:00:00+08:00", { originalPath: "E:/OtherPC/a.jpg" })],
      restorable: 0,
      synced: 2,
      home: "C:/Steam/shots",
    });
    api.shotDiff.mockResolvedValue({ gameId: "g1", new: [], duplicates: 0, ignored: [], missing: [], restorable: 0, synced: 3, home: "C:/Steam/shots" });
    api.syncShotsToDisk.mockResolvedValue({ written: 1, bytes: 1000, exists: 0, noDir: 0, dirs: ["C:/Steam/shots"] });
    render(<ShotDiffDialog gameId="g1" gameName="空洞骑士" onClose={() => {}} onChanged={() => {}} />);
    expect(await screen.findByText(/1 张的原文件夹不在本机（例如从其他电脑导入的）/)).toBeInTheDocument();
    expect(screen.getByText("磁盘上没有未收集的新截图")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "写回磁盘（1）" }));
    await waitFor(() => expect(api.syncShotsToDisk).toHaveBeenCalledWith("g1", ["b3:777777"]));
    expect(await screen.findByText("图库与磁盘一致：3 张截图两边都有")).toBeInTheDocument();
  });
});

describe("其他", () => {
  it("第一次关闭窗口：选择退出并记住", async () => {
    render(<CloseDialog onClose={() => {}} />);
    expect(screen.getByRole("radio", { name: /最小化到托盘（推荐）/ })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: /退出篝火/ }));
    fireEvent.click(screen.getByRole("button", { name: "退出" }));
    await waitFor(() => expect(api.closeWindow).toHaveBeenCalledWith(false, true));
  });

  it("发现页一键加入全部只有截图的游戏", async () => {
    useStore.setState({ scan: { platforms: ["steam"], rules: { path: "x", games: 1, userGames: 0 }, games: [], withoutSaves: [] }, scanning: false, toasts: [] });
    api.shotCandidates.mockResolvedValue([
      { key: "steam:504230", name: "Celeste", source: "steam", appId: "504230", count: 2, dir: "C:/s" },
      { key: "capture:Minishoot", name: "Minishoot", source: "capture", appId: null, count: 1, dir: "C:/c" },
    ]);
    api.addShotsGame.mockImplementation(async (_k: string, name: string) => ({ id: name, name, platform: "custom", enabled: true }));
    api.gameRow.mockResolvedValue(row());
    render(<ScanPage />);
    fireEvent.click(await screen.findByRole("button", { name: /全部加入（2）/ }));
    await waitFor(() => expect(api.addShotsGame).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(useStore.getState().toasts.map((t) => t.text)).toContain("已加入 2 个只管理截图的游戏"));
  });

  it("启动页面可由地址指定", () => {
    expect(initialView("")).toEqual({ name: "library" });
    expect(initialView("#/gallery")).toEqual({ name: "gallery" });
    expect(initialView("#/game/steam-367520")).toEqual({ name: "game", id: "steam-367520" });
    expect(initialView("#/nope")).toEqual({ name: "library" });
  });
});

describe("U-21 底栏与图库自动备份", () => {
  it("底栏：所选游戏显示数量，全选框与反选一组，添加 / 导入与导出在一起", async () => {
    seed([row({ id: "a", name: "Alpha", nameZh: null }), row({ id: "b", name: "Beta", nameZh: null, state: "up-to-date" })]);
    render(<Library />);
    expect(screen.getByRole("button", { name: "所选游戏" })).toHaveTextContent("已选 1 个");
    const group = screen.getByRole("group", { name: "选择" });
    const all = within(group).getByRole("checkbox", { name: "全选" });
    expect((all as HTMLInputElement).indeterminate).toBe(true);
    fireEvent.click(all);
    expect(useStore.getState().selected).toEqual(new Set(["a", "b"]));
    fireEvent.click(within(group).getByRole("button", { name: "反选" }));
    expect(useStore.getState().selected).toEqual(new Set());
    fireEvent.click(within(group).getByRole("button", { name: "反选" }));
    expect(useStore.getState().selected).toEqual(new Set(["a", "b"]));
    const tools = screen.getByRole("group", { name: "添加、导入与导出" });
    expect(within(tools).getByRole("button", { name: "添加 / 导入" })).toBeInTheDocument();
    expect(within(tools).getByRole("button", { name: "导出" })).toBeInTheDocument();
  });

  it("图库：截图自动备份快捷设置，选择定时收集后保存", async () => {
    useStore.setState({ info, toasts: [] });
    api.gallery.mockResolvedValue({ games: [], shots: [], trash: [], bytes: 0, trashBytes: 0, dir: "D:/Repo/screenshots" });
    api.saveSettings.mockResolvedValue(info);
    render(<Gallery />);
    const btn = await screen.findByRole("button", { name: /截图自动备份/ });
    expect(btn).toHaveTextContent("截图：随备份收集");
    fireEvent.click(btn);
    fireEvent.click(screen.getByRole("menuitemradio", { name: "每天 21:00" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalled());
    expect(api.saveSettings.mock.calls[0][0].shots).toMatchObject({ enabled: true, schedule: "daily" });
    await waitFor(() => expect(btn).toHaveTextContent("自动收集：每天 21:00"));
    // 截图变化后自动收集，并选择检查间隔。
    fireEvent.click(btn);
    fireEvent.click(screen.getByRole("menuitemradio", { name: /截图变化后自动收集/ }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledTimes(2));
    expect(api.saveSettings.mock.calls[1][0].shots).toMatchObject({ schedule: "daily", on_change: true, check_minutes: 5 });
    await waitFor(() => expect(btn).toHaveTextContent("自动收集：每天 21:00 + 变化后（每 5 分钟检查）"));
    // 更多设置：跳到设置中的截图备份。
    fireEvent.click(btn);
    fireEvent.click(screen.getByRole("menuitem", { name: /更多设置/ }));
    expect(useStore.getState().view).toEqual({ name: "settings", section: "shots" });
  });
});

describe("U-27 / U-28", () => {
  it("设置：截图来源说明，添加与移除公用截图文件夹", async () => {
    useStore.setState({ info, toasts: [], view: { name: "settings" } });
    dialog.open.mockResolvedValue("D:/Screenshots");
    api.saveSettings.mockResolvedValue(info);
    render(<Settings />);
    expect(await screen.findByText(/C:\/Steam\/userdata\\<账号>\\760\\remote\\<AppID>\\screenshots/)).toBeInTheDocument();
    expect(screen.getByText(/按文件名开头的窗口标题对应游戏/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "添加公用截图文件夹…" }));
    expect(await screen.findByText("D:/Screenshots")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalled());
    expect(api.saveSettings.mock.calls[0][0].shots.shared_dirs).toEqual(["D:/Screenshots"]);
    fireEvent.click(screen.getByRole("button", { name: "移除 D:/Screenshots" }));
    expect(screen.queryByText("D:/Screenshots")).toBeNull();
  });

  it("图库：分组可折叠，一键全部折叠 / 展开并记住", async () => {
    localStorage.removeItem("bonfire.gallery.folded");
    api.gallery.mockResolvedValue({
      games: [
        { id: "g1", name: "Hollow Knight", nameZh: null, platform: "steam", inLibrary: true, mode: "both" },
        { id: "g2", name: "Celeste", nameZh: null, platform: "steam", inLibrary: true, mode: "both" },
      ],
      shots: [shot("g1", "b3:111111", "2026-10-09T21:00:00+08:00"), shot("g2", "b3:222222", "2026-10-08T21:00:00+08:00")],
      trash: [],
      bytes: 2000,
      trashBytes: 0,
      dir: "D:/Repo/screenshots",
    });
    const { container } = render(<Gallery />);
    await screen.findByText(/2 张截图/);
    expect(container.querySelectorAll("[data-key]")).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "折叠「Hollow Knight」" }));
    expect(container.querySelectorAll("[data-key]")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "展开「Hollow Knight」" })).toHaveAttribute("aria-expanded", "false");
    // 折叠后显示日期范围与叠放的缩略图。
    expect(container.querySelectorAll(".shot-day.folded .fold-photo")).toHaveLength(1);
    expect(container.querySelector(".shot-day.folded .fold-range")?.textContent).toMatch(/10/);
    fireEvent.click(screen.getByRole("button", { name: "全部折叠" }));
    expect(container.querySelectorAll("[data-key]")).toHaveLength(0);
    expect(localStorage.getItem("bonfire.gallery.folded")).toContain("Hollow Knight".length ? "game:" : "");
    fireEvent.click(screen.getByRole("button", { name: "全部展开" }));
    expect(container.querySelectorAll("[data-key]")).toHaveLength(2);
    localStorage.removeItem("bonfire.gallery.folded");
  });
});

describe("U-29 未识别的截图", () => {
  it("图库中单独成组，可把所选归到游戏库中的游戏", async () => {
    seed([row({ id: "g1", name: "Hollow Knight", nameZh: null })]);
    api.gallery.mockResolvedValue({
      games: [{ id: "unsorted", name: "未识别的截图", nameZh: null, platform: "unsorted", inLibrary: false, mode: "shots" }],
      shots: [shot("unsorted", "b3:111111", "2026-10-09T21:00:00+08:00"), shot("unsorted", "b3:222222", "2026-10-08T21:00:00+08:00")],
      trash: [],
      bytes: 2000,
      trashBytes: 0,
      dir: "D:/Repo/screenshots",
    });
    api.assignUnsorted.mockResolvedValue(2);
    render(<Gallery />);
    expect(await screen.findByText(/没有对应到游戏库中的游戏，已照常备份/)).toBeInTheDocument();
    expect(screen.queryByText("已移出游戏库")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "全部归到游戏…（2）" }));
    const dlg = await screen.findByRole("dialog", { name: "把 2 张截图归到游戏" });
    expect(within(dlg).getByLabelText("归到")).toHaveValue("g1");
    fireEvent.click(within(dlg).getByRole("button", { name: "归到这个游戏" }));
    await waitFor(() => expect(api.assignUnsorted).toHaveBeenCalledWith(["b3:111111", "b3:222222"], "g1"));
    expect(useStore.getState().toasts.map((x) => x.text)).toContain("已把 2 张截图归到「Hollow Knight」");
  });
});

describe("U-33 记住游戏库的筛选", () => {
  it("筛选与管理方式写入本机，重新打开时恢复；无效的值回到全部", async () => {
    localStorage.removeItem("bonfire.library.filters");
    seed([row({ id: "a", name: "Alpha", nameZh: null, mode: "shots" })]);
    render(<Library />);
    fireEvent.click(within(screen.getByRole("group", { name: "按管理方式筛选" })).getByRole("button", { name: /只管理截图/ }));
    const saved = JSON.parse(localStorage.getItem("bonfire.library.filters") ?? "{}");
    expect(saved).toMatchObject({ modeFilter: "shots", stateFilter: "all", platformFilter: "all", profileFilter: null });
    expect(readFilters()).toMatchObject({ modeFilter: "shots" });
    resetUiStateForTest();
    localStorage.setItem("bonfire.library.filters", JSON.stringify({ modeFilter: "bogus", stateFilter: "has-changes", profileFilter: { name: "周末", ids: ["a", 3] } }));
    expect(readFilters()).toEqual({ modeFilter: "all", stateFilter: "has-changes", platformFilter: "all", profileFilter: { name: "周末", ids: ["a"] }, newOnly: false });
    localStorage.removeItem("bonfire.library.filters");
  });
});

describe("U-34 新加入的游戏", () => {
  it("第一次使用时已有的不算；之后出现的标为新加入，可筛选、打开或全部标为已看过", async () => {
    localStorage.removeItem("bonfire.library.seen");
    expect(noteGames(["a", "b"], 1_000)).toEqual({});
    expect(noteGames(["a", "b", "c"], 2_000)).toEqual({ c: 2_000 });
    // 超过 7 天不再算新加入。
    expect(noteGames(["a", "b", "c"], 2_000 + 8 * 86_400_000)).toEqual({});
    // 移出游戏库后重新加入：同样标为新加入。
    expect(noteGames(["a", "c"], 3_000)).toEqual({ c: 2_000 });
    expect(noteGames(["a", "b", "c"], 4_000)).toEqual({ b: 4_000, c: 2_000 });

    seed([row({ id: "a", name: "Alpha", nameZh: null }), row({ id: "c", name: "Gamma", nameZh: null })]);
    useStore.setState({ fresh: { c: Date.now() } });
    render(<Library />);
    expect(screen.getAllByText("新加入")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: /按筛选方案、状态与平台筛选/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: "只看新加入的（1）" }));
    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.getByRole("button", { name: /按筛选方案、状态与平台筛选/ })).toHaveTextContent("筛选：新加入");
    // 打开详情后不再标记。
    useStore.getState().go({ name: "game", id: "c" });
    expect(useStore.getState().fresh).toEqual({});
    expect(useStore.getState().newOnly).toBe(false);
    localStorage.removeItem("bonfire.library.seen");
    localStorage.removeItem("bonfire.library.filters");
  });
});

describe("U-38 界面状态由后台保存", () => {
  it("修改筛选时交给后台写盘；读入后台状态后恢复筛选", async () => {
    localStorage.removeItem("bonfire.library.filters");
    seed([row({ id: "a", name: "Alpha", nameZh: null, mode: "shots" })]);
    render(<Library />);
    fireEvent.click(within(screen.getByRole("group", { name: "按管理方式筛选" })).getByRole("button", { name: /只管理截图/ }));
    expect(api.uiStateSet).toHaveBeenCalledWith("bonfire.library.filters", expect.stringContaining('"modeFilter":"shots"'));
    // 浏览器存储被清空（例如没来得及写盘），后台保存的状态仍能恢复。
    localStorage.removeItem("bonfire.library.filters");
    api.uiStateGet.mockResolvedValue({ "bonfire.library.filters": JSON.stringify({ stateFilter: "has-changes", modeFilter: "saves" }) });
    useStore.setState({ stateFilter: "all", modeFilter: "all" });
    await loadUiState();
    useStore.getState().restoreFilters();
    expect(useStore.getState()).toMatchObject({ stateFilter: "has-changes", modeFilter: "saves" });
    resetUiStateForTest();
    api.uiStateGet.mockResolvedValue({});
  });
});

describe("U-39 记住勾选的游戏", () => {
  it("勾选变化时保存，重新加载游戏库时恢复；没有保存过时按默认勾选", async () => {
    localStorage.removeItem("bonfire.library.selected");
    seed([row({ id: "a", name: "Alpha", nameZh: null }), row({ id: "b", name: "Beta", nameZh: null, state: "up-to-date" })]);
    useStore.getState().toggle("b");
    expect(JSON.parse(localStorage.getItem("bonfire.library.selected") ?? "[]").sort()).toEqual(["a", "b"]);
    expect(api.uiStateSet).toHaveBeenCalledWith("bonfire.library.selected", expect.any(String));
    useStore.getState().toggle("a");
    api.loadLibrary.mockResolvedValue([row({ id: "a", name: "Alpha", nameZh: null }), row({ id: "b", name: "Beta", nameZh: null, state: "up-to-date" })]);
    useStore.setState({ selected: new Set(), loading: false });
    localStorage.setItem("bonfire.library.selected", JSON.stringify(["b", "gone"]));
    resetUiStateForTest();
    await useStore.getState().loadLibrary(false);
    expect(useStore.getState().selected).toEqual(new Set(["b"]));
    localStorage.removeItem("bonfire.library.selected");
  });
});

describe("U-40 是否记住上次的选择", () => {
  it("关闭后清除已保存的筛选与勾选，之后不再保存，启动时用默认", async () => {
    seed([row({ id: "a", name: "Alpha", nameZh: null, mode: "shots" })]);
    useStore.getState().setModeFilter("shots");
    expect(localStorage.getItem("bonfire.library.filters")).toContain("shots");
    useStore.getState().setInfo({ ...info, rememberFilters: false, rememberSelection: false });
    expect(localStorage.getItem("bonfire.library.filters")).toBeNull();
    expect(api.uiStateSet).toHaveBeenCalledWith("bonfire.library.filters", null);
    useStore.getState().setStateFilter("has-changes");
    expect(localStorage.getItem("bonfire.library.filters")).toBeNull();
    useStore.getState().toggle("a");
    expect(localStorage.getItem("bonfire.library.selected")).toBeNull();
    useStore.getState().restoreFilters();
    expect(useStore.getState()).toMatchObject({ stateFilter: "all", modeFilter: "all" });
    useStore.getState().setInfo(info);
  });

  it("设置页：界面分组中可以关闭记住筛选与勾选", async () => {
    useStore.setState({ info, toasts: [], view: { name: "settings" } });
    api.saveSettings.mockResolvedValue(info);
    render(<Settings />);
    const box = await screen.findByRole("checkbox", { name: /游戏库的筛选与管理方式/ });
    expect(box).toBeChecked();
    fireEvent.click(box);
    fireEvent.click(screen.getByRole("checkbox", { name: /游戏库中勾选的游戏/ }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalled());
    expect(api.saveSettings.mock.calls[0][0].general).toMatchObject({ remember_filters: false, remember_selection: false });
  });
});

describe("M4-4 历史清理", () => {
  it("设置保留规则并保存；立即清理先预览，确认后执行", async () => {
    useStore.setState({ info, toasts: [], view: { name: "settings" } });
    api.saveSettings.mockResolvedValue(info);
    const plan = { games: [{ gameId: "g", name: "Alpha", snapshots: 9, removed: ["1", "2", "3"], rewritten: 2 }], removed: 3, gc: { chunks: 3, blobs: 3, trees: 3, snapshots: [], bytes: 3 * 1024 * 1024, tmpFiles: 0, sources: [{ kind: "planned", objects: 6, bytes: 2 * 1024 * 1024 }, { kind: "shots", objects: 2, bytes: 1024 * 1024 }], dryRun: true }, dryRun: true };
    api.cleanHistory.mockImplementation(async (dry: boolean) => ({ ...plan, dryRun: dry }));
    render(<Settings />);
    // 规则说明默认展开，按当前设置写出数字。
    expect(await screen.findByText("清理规则说明（清理前请先阅读）")).toBeInTheDocument();
    expect(screen.getByText(/最近的 20 个；最近 14 天里每天最新的 1 个；最近 8 周里每周最新的 1 个/)).toBeInTheDocument();
    // 开启自动清理先确认：取消则不开启，确认后才勾上。
    dialog.ask.mockResolvedValueOnce(false);
    fireEvent.click(await screen.findByRole("checkbox", { name: /每次自动备份后按下面的规则清理/ }));
    await waitFor(() => expect(dialog.ask).toHaveBeenCalled());
    expect(String(dialog.ask.mock.calls.at(-1)![0])).toContain("保留最近 20 个、最近 14 天每天 1 个、最近 8 周每周 1 个");
    expect(screen.getByRole("checkbox", { name: /每次自动备份后按下面的规则清理/ })).not.toBeChecked();
    dialog.ask.mockResolvedValueOnce(true);
    fireEvent.click(screen.getByRole("checkbox", { name: /每次自动备份后按下面的规则清理/ }));
    await waitFor(() => expect(screen.getByRole("checkbox", { name: /每次自动备份后按下面的规则清理/ })).toBeChecked());
    expect(screen.getByText(/不再逐次询问。$/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("spinbutton", { name: "保留最近的快照数" }), { target: { value: "5" } });
    // 有未保存的修改时不能清理。
    expect(screen.getByRole("button", { name: "立即清理…" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalled());
    expect(api.saveSettings.mock.calls.at(-1)![0].retention).toMatchObject({ enabled: true, keep_last: 5, keep_daily: 14, keep_weekly: 8, scope: "auto" });

    await waitFor(() => expect(screen.getByRole("button", { name: "立即清理…" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "立即清理…" }));
    const dlg = await screen.findByRole("dialog", { name: "清理历史" });
    await within(dlg).findByText("个快照将被清理");
    expect(api.cleanHistory).toHaveBeenCalledWith(true);
    expect(within(dlg).getByText("3.0 MB")).toBeInTheDocument();
    expect(within(dlg).getByText("3 / 9 个快照")).toBeInTheDocument();
    // 空间来源明细。
    expect(within(dlg).getByText("可释放的空间来自")).toBeInTheDocument();
    expect(within(dlg).getByText("早期版本留下的截图")).toBeInTheDocument();
    expect(within(dlg).getByText("2.0 MB")).toBeInTheDocument();
    // 安全提示：勾选确认前不能清理。
    expect(within(dlg).getByText("删除后无法撤销")).toBeInTheDocument();
    expect(within(dlg).getByRole("button", { name: "清理" })).toBeDisabled();
    fireEvent.click(within(dlg).getByRole("checkbox", { name: /我已核对/ }));
    fireEvent.click(within(dlg).getByRole("button", { name: "清理" }));
    await within(dlg).findByText("个快照已清理");
    expect(api.cleanHistory).toHaveBeenCalledWith(false);
    fireEvent.click(within(dlg).getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("M4-2 游戏程序", () => {
  it("显示安装目录，添加、去重与移除进程名后保存", async () => {
    useStore.setState({ toasts: [] });
    api.gameProcesses.mockResolvedValue(["RetroArch.exe"]);
    api.setGameProcesses.mockResolvedValue([]);
    dialog.open.mockResolvedValue("C:/Games/Alpha/bin/Alpha.exe");
    const onClose = vi.fn();
    render(<ProcessDialog gameId="g" name="Alpha" installDir={null} onClose={onClose} />);
    const dlg = await screen.findByRole("dialog", { name: "「Alpha」的游戏程序" });
    await within(dlg).findByText("RetroArch.exe");
    expect(within(dlg).getByText("没有安装目录")).toBeInTheDocument();
    const box = within(dlg).getByRole("textbox", { name: "程序文件名" });
    fireEvent.change(box, { target: { value: "retroarch" } });
    fireEvent.keyDown(box, { key: "Enter" });
    expect(within(dlg).getAllByRole("listitem")).toHaveLength(1);
    fireEvent.click(within(dlg).getByRole("button", { name: /选择程序/ }));
    await within(dlg).findByText("Alpha.exe");
    fireEvent.click(within(dlg).getByRole("button", { name: "移除 RetroArch.exe" }));
    fireEvent.click(within(dlg).getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.setGameProcesses).toHaveBeenCalledWith("g", ["Alpha.exe"]));
    expect(onClose).toHaveBeenCalled();
  });
});

describe("多选删除快照", () => {
  it("先检查：列出不能删除的原因与删除后的影响，勾选确认后只删除可以删的", async () => {
    useStore.setState({ toasts: [] });
    const node = (id: string, comment: string, createdAt: string) => ({ id, parents: [], createdAt, trigger: "manual", comment, files: 1, refs: [], host: "pc" }) as unknown as import("./types").SnapshotNode;
    const nodes = [node("aaa", "第一章", "2026-10-01T10:00:00+08:00"), node("bbb", "Boss 前", "2026-10-02T10:00:00+08:00"), node("ccc", "第二章", "2026-10-03T10:00:00+08:00")];
    api.deleteSnapshots.mockImplementation(async (_g: string, ids: string[], dry: boolean) => ({
      removed: ids.filter((i) => i !== "bbb"),
      blocked: dry ? [{ id: "bbb", reason: "tag", detail: "boss" }] : [],
      rewritten: 1,
      branches: [
        { name: "main", to: "aaa", toCreatedAt: "2026-10-01T10:00:00+08:00" },
        { name: "支线", to: null, toCreatedAt: null },
      ],
      sources: [{ kind: "planned", objects: 3, bytes: 2048 }],
      headBranch: null,
      currentRemoved: true,
      bytes: 2048,
      dryRun: dry,
    }));
    const onDone = vi.fn();
    render(<DeleteSnapshotsDialog gameId="g" ids={["bbb", "ccc"]} nodes={nodes} onClose={() => {}} onDone={onDone} />);
    const dlg = await screen.findByRole("dialog", { name: "删除所选的 2 个快照" });
    await within(dlg).findByText("1 个快照不能删除，会跳过");
    expect(within(dlg).getByText("带有标签「boss」，先删除标签才能删除")).toBeInTheDocument();
    expect(within(dlg).getByText("路线「main」会变短")).toBeInTheDocument();
    expect(within(dlg).getByText(/路线退回到 .*「第一章」/)).toBeInTheDocument();
    expect(within(dlg).getByText("路线「支线」会被删除")).toBeInTheDocument();
    expect(within(dlg).getByText("这次删除的快照")).toBeInTheDocument();
    expect(within(dlg).getByText(/当前存档对应的快照会被删除/)).toBeInTheDocument();
    const del = within(dlg).getByRole("button", { name: /删除 1 个快照/ });
    expect(del).toBeDisabled();
    fireEvent.click(within(dlg).getByRole("checkbox", { name: /我已确认要删除这 1 个快照/ }));
    // 有路线会被删除时还要单独确认。
    expect(del).toBeDisabled();
    fireEvent.click(within(dlg).getByRole("checkbox", { name: "我知道路线「支线」会被删除" }));
    fireEvent.click(del);
    await waitFor(() => expect(api.deleteSnapshots).toHaveBeenLastCalledWith("g", ["ccc"], false));
    expect(onDone).toHaveBeenCalled();
  });
});

describe("M5-2 仓库加密", () => {
  it("解锁页：密码不对时提示，正确后进入", async () => {
    const { UnlockScreen } = await import("./components/Security");
    api.repoUnlock.mockRejectedValueOnce({ kind: "usage", message: "密码不对" });
    api.repoUnlock.mockResolvedValueOnce(undefined);
    const onUnlocked = vi.fn();
    render(<UnlockScreen onUnlocked={onUnlocked} />);
    const box = screen.getByLabelText("密码");
    fireEvent.change(box, { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: /解锁/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("密码不对");
    fireEvent.change(box, { target: { value: "right one" } });
    fireEvent.click(screen.getByRole("button", { name: /解锁/ }));
    await waitFor(() => expect(onUnlocked).toHaveBeenCalled());
    expect(api.repoUnlock).toHaveBeenLastCalledWith("right one", true);
  });

  it("加密仓库：两次密码一致且确认风险后才能加密", async () => {
    const { SecuritySection } = await import("./components/Security");
    useStore.setState({ toasts: [] });
    api.repoEncrypt.mockResolvedValue(undefined);
    render(<SecuritySection />);
    fireEvent.click(await screen.findByRole("button", { name: /加密仓库…/ }));
    const dlg = await screen.findByRole("dialog", { name: "加密仓库" });
    const ok = within(dlg).getByRole("button", { name: "加密仓库" });
    fireEvent.change(within(dlg).getByLabelText("新密码（至少 6 个字符）"), { target: { value: "secret1" } });
    fireEvent.change(within(dlg).getByLabelText("再输入一次"), { target: { value: "secret2" } });
    expect(within(dlg).getByText("两次输入的密码不一样")).toBeInTheDocument();
    fireEvent.change(within(dlg).getByLabelText("再输入一次"), { target: { value: "secret1" } });
    expect(ok).toBeDisabled();
    fireEvent.click(within(dlg).getByRole("checkbox", { name: /我已记下密码/ }));
    fireEvent.click(ok);
    await waitFor(() => expect(api.repoEncrypt).toHaveBeenCalledWith("secret1", true));
  });
});
