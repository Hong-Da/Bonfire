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
import { useStore, defaultSelection, initialView } from "./store";
import Library from "./pages/Library";
import Gallery from "./pages/Gallery";
import ScanPage from "./pages/ScanPage";
import ShotDiffDialog from "./pages/ShotDiffDialog";
import CloseDialog from "./components/CloseDialog";
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
  });
});
