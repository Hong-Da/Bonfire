import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { sharedApi as api, history, row } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: () => undefined };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("./lib/confirm", () => ({ confirm: (...a: unknown[]) => dialog.ask(...a), ConfirmHost: () => null }));

import i18n from "./i18n";
import { useStore, defaultSelection } from "./store";
import BackupDialog from "./pages/BackupDialog";
import GameDetail from "./pages/GameDetail";
import Settings from "./pages/Settings";
import ScanPage from "./pages/ScanPage";
import AddGameDialog from "./pages/AddGameDialog";
import Timeline from "./components/Timeline";
import type { Config, GameRow } from "./types";

function seed(rows: GameRow[]) {
  useStore.setState({
    rows: Object.fromEntries(rows.map((r) => [r.id, r])),
    order: rows.map((r) => r.id),
    selected: defaultSelection(rows),
    toasts: [],
    undoable: {},
    info: null,
  });
}

beforeEach(async () => {
  vi.clearAllMocks();
  dialog.ask.mockResolvedValue(true);
  useStore.setState({ toasts: [] });
  await i18n.changeLanguage("zh-CN");
});

describe("备份确认", () => {
  it("分步：选择存档与截图，依次确认，确认页可移除游戏；已是最新的跳过存档", async () => {
    const changed = row({ id: "a", name: "A", nameZh: null });
    const latest = row({ id: "b", name: "B", nameZh: null, state: "up-to-date" });
    const savesOnly = row({ id: "d", name: "D", nameZh: null, mode: "saves" });
    const shotsOnly = row({ id: "c", name: "C", nameZh: null, state: "shots-only", shotsOnly: true, mode: "shots" });
    seed([changed, latest, savesOnly, shotsOnly]);
    api.runBackup.mockResolvedValue([
      { gameId: "a", ok: true, result: { result: "no-changes" }, error: null, shots: 0, shotsOnly: false },
      { gameId: "c", ok: true, result: null, error: null, shots: 2, shotsOnly: true },
    ]);
    api.album.mockImplementation(async (id: string) => ({ shots: [], trash: [], trashBytes: 0, dir: "D:/x", shotsOnly: false, enabled: true, followsGlobal: true, globalEnabled: true, dirs: [], pending: id === "a" ? 4 : id === "c" ? 2 : 0, bytes: 0, sources: [] }));
    render(<BackupDialog rows={[changed, latest, savesOnly, shotsOnly]} onClose={() => {}} />);
    // 第 1 步：两种内容默认都选。
    expect(screen.getByText("第 1 步，共 3 步：选择内容")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: /备份存档/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /备份截图/ })).toBeChecked();
    expect(screen.getByText(/2 个游戏的存档有变化或还没备份；1 个已是最新，会跳过/)).toBeInTheDocument();
    expect(screen.getByText(/从 3 个游戏收集新截图/)).toBeInTheDocument();
    expect(screen.getByText(/1 个只管理截图，不备份存档/)).toBeInTheDocument();
    expect(screen.getByText(/1 个只管理存档，不收集截图/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    // 第 2 步：确认存档，移除 D。
    expect(screen.getByText("已是最新，将跳过（1）")).toBeInTheDocument();
    // 只管理截图的游戏单独列出：不备份存档。
    expect(screen.getByRole("region", { name: "只管理截图，不备份存档（1）" })).toHaveTextContent("只管理截图");
    fireEvent.click(screen.getByRole("button", { name: "这次不备份「D」" }));
    expect(screen.queryByLabelText("「D」的备注")).toBeNull();
    // 移出的游戏列在下方，可以恢复。
    fireEvent.click(screen.getByRole("button", { name: "恢复「D」" }));
    expect(screen.getByLabelText("「D」的备注")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "这次不备份「D」" }));
    expect(screen.getByText("这次不备份（1）：")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    // 第 3 步：确认截图；没有新截图的 B 单独列出并跳过。
    expect(screen.getByText("第 3 步，共 3 步：确认截图")).toBeInTheDocument();
    // 每个游戏显示已有与本次新增的数量（按磁盘重新数）。
    expect(await screen.findByText("本次新增 4 张")).toBeInTheDocument();
    expect(await screen.findByText("本次将从 2 个游戏新增 6 张截图")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "只管理存档，不收集截图（1）" })).toHaveTextContent("只管理存档");
    expect(screen.getByRole("region", { name: "没有新截图，将跳过（1）" })).toHaveTextContent("B");
    expect(screen.queryByRole("button", { name: "这次不备份「B」" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "备份 2 个游戏" }));
    await waitFor(() => expect(api.runBackup).toHaveBeenCalled());
    expect(api.runBackup.mock.calls[0][0]).toEqual([
      { gameId: "a", comment: undefined, tag: undefined },
      { gameId: "c", comment: undefined, tag: undefined, saves: false },
    ]);
    expect(await screen.findByText("收集了 2 张新截图")).toBeInTheDocument();
  });

  it("只选截图时跳过存档确认页，只收集截图", async () => {
    const a = row({ id: "a", name: "A", nameZh: null });
    seed([a]);
    api.runBackup.mockResolvedValue([]);
    api.album.mockResolvedValue({ shots: [], trash: [], trashBytes: 0, dir: "D:/x", shotsOnly: false, enabled: true, followsGlobal: true, globalEnabled: true, dirs: [], pending: 3, bytes: 0, sources: [] });
    render(<BackupDialog rows={[a]} onClose={() => {}} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /备份存档/ }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    expect(screen.getByText("第 2 步，共 2 步：确认截图")).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "备份 1 个游戏" }));
    await waitFor(() => expect(api.runBackup).toHaveBeenCalled());
    expect(api.runBackup.mock.calls[0][0]).toEqual([{ gameId: "a", comment: undefined, tag: undefined, saves: false }]);
  });

  it("游戏正在运行时提醒，但不阻止备份", async () => {
    const r = row();
    seed([r]);
    api.runningGames.mockResolvedValue([{ gameId: r.id, processes: ["hollow_knight.exe"] }]);
    api.runBackup.mockResolvedValue([]);
    render(<BackupDialog rows={[r]} onClose={() => {}} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /备份截图/ }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    expect(await screen.findByText(/正在运行：空洞骑士（hollow_knight.exe）/)).toBeInTheDocument();
    const btn = screen.getByRole("button", { name: "备份 1 个游戏" });
    expect(btn).toBeEnabled();
    fireEvent.click(btn);
    await waitFor(() => expect(api.runBackup).toHaveBeenCalled());
  });

  it("全部已是最新时不能备份并说明原因", () => {
    const latest = row({ state: "up-to-date" });
    seed([latest]);
    render(<BackupDialog rows={[latest]} onClose={() => {}} />);
    expect(screen.getByText("所选游戏的存档都已是最新；1 个已是最新，会跳过")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: /备份截图/ }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    expect(screen.getByText("所选的 1 个游戏都已是最新，不需要备份。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "备份 0 个游戏" })).toBeDisabled();
  });
});

describe("分支图", () => {
  it("只有一条路线时也用彩色，不用灰色", () => {
    const { container } = render(<Timeline nodes={history().nodes} head="b3:cccccccc" selected={null} onSelect={() => {}} />);
    const strokes = [...container.querySelectorAll("path[stroke]")].map((p) => p.getAttribute("stroke"));
    expect(strokes.length).toBeGreaterThan(0);
    expect(strokes.every((s) => s?.startsWith("var(--lane-"))).toBe(true);
  });

  it("详情页可放大查看完整分支图，双击快照回到详情并选中它", async () => {
    seed([row()]);
    api.history.mockResolvedValue(history());
    render(<GameDetail id="steam-367520" />);
    fireEvent.click(await screen.findByRole("button", { name: /放大查看/ }));
    const viewer = screen.getByRole("dialog", { name: /分支图 · 空洞骑士/ });
    const target = within(viewer).getAllByRole("option").find((o) => o.textContent?.includes("打完螳螂领主"))!;
    fireEvent.click(target);
    expect(within(viewer).getByRole("button", { name: "查看这个快照" })).toBeInTheDocument();
    fireEvent.doubleClick(target);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /分支图 · 空洞骑士/ })).toBeNull());
    expect(screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true")?.textContent).toContain("打完螳螂领主");
  });
});

describe("只管理截图", () => {
  it("详情页默认打开截图页，存档页显示未管理，可恢复管理存档", async () => {
    seed([row({ state: "shots-only", shotsOnly: true, mode: "shots", shots: 3 })]);
    api.history.mockResolvedValue({ ...history(), nodes: [], head: null, refs: [] });
    render(<GameDetail id="steam-367520" />);
    expect(await screen.findByRole("tab", { name: /截图/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /存档/ })).toHaveTextContent("未管理存档");
    expect(screen.getByText(/这个游戏只管理截图，不备份存档/)).toBeInTheDocument();
    // 页头的“管理方式”按钮：改为存档和截图。
    fireEvent.click(screen.getByRole("button", { name: /只管理截图/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /存档和截图/ }));
    await waitFor(() => expect(api.setManageMode).toHaveBeenCalledWith(["steam-367520"], "both"));
  });

  it("发现页列出有截图、不在游戏库中的游戏，可加入为只管理截图", async () => {
    useStore.setState({ scan: { platforms: ["steam"], rules: { path: "x", games: 1, userGames: 0 }, games: [], withoutSaves: [] }, scanning: false, toasts: [] });
    api.shotCandidates.mockResolvedValue([{ key: "capture:Minishoot", name: "Minishoot", source: "capture", appId: null, count: 2, dir: "C:/Captures" }]);
    api.addShotsGame.mockResolvedValue({ id: "custom-1", name: "Minishoot", platform: "custom", enabled: true });
    api.gameRow.mockResolvedValue(row({ id: "custom-1", name: "Minishoot", shotsOnly: true, state: "shots-only" }));
    render(<ScanPage />);
    expect(await screen.findByText("有截图、但不在游戏库中的游戏（1）")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /加入（只管理截图）/ }));
    await waitFor(() => expect(api.addShotsGame).toHaveBeenCalledWith("capture:Minishoot", "Minishoot", []));
    await waitFor(() => expect(useStore.getState().toasts.map((t) => t.text)).toContain("已加入「Minishoot」，只管理截图"));
  });

  it("手动添加游戏时可以只管理截图，不需要存档目录", async () => {
    seed([]);
    api.addShotsGame.mockResolvedValue({ id: "custom-2", name: "My Game", platform: "custom", enabled: true });
    api.gameRow.mockResolvedValue(row({ id: "custom-2", name: "My Game" }));
    dialog.open.mockResolvedValue("D:/Shots");
    render(<AddGameDialog onClose={() => {}} />);
    fireEvent.click(screen.getByRole("radio", { name: /只管理截图/ }));
    fireEvent.change(screen.getByLabelText("游戏名称"), { target: { value: "My Game" } });
    fireEvent.click(screen.getByRole("button", { name: /添加文件夹/ }));
    await screen.findByText("D:/Shots");
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    await waitFor(() => expect(api.addShotsGame).toHaveBeenCalledWith(null, "My Game", ["D:/Shots"]));
  });
});

describe("自动备份设置", () => {
  it("定时备份与存档变化后自动备份写入配置", async () => {
    const cfg = {
      general: { theme: "auto", close_to_tray: true },
      backup: { include_config_files: true, size_warning_gb: 2, disabled_platforms: [], protect_local: true },
      shots: { enabled: true, schedule: "off", at: "21:00", weekday: 7, every_hours: 6 },
      metadata: { online: true, igdb_client_id: "", igdb_client_secret: "", steamgriddb_api_key: "" },
      log: { level: "info", keep_days: 14 },
    } as unknown as Config;
    api.getSettings.mockResolvedValue(cfg);
    api.saveSettings.mockResolvedValue({});
    api.autoStatus.mockResolvedValue({ lastScheduled: "2026-10-09T21:00:00+08:00", lastCheck: null });
    render(<Settings />);
    fireEvent.change(await screen.findByLabelText("定时备份"), { target: { value: "weekly" } });
    fireEvent.change(screen.getByLabelText("星期"), { target: { value: "5" } });
    fireEvent.click(screen.getByRole("checkbox", { name: /定期检查存档/ }));
    fireEvent.change(screen.getByLabelText("存档停止写入"), { target: { value: "3" } });
    expect(screen.getByText(/上次定时备份/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalled());
    expect(api.saveSettings.mock.calls[0][0].auto_backup).toEqual({ schedule: "weekly", at: "21:00", weekday: 5, every_hours: 6, on_change: true, check_minutes: 5, quiet_minutes: 3 });
  });
});
