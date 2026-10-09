import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { sharedApi as api, row, history } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), errorKind: () => "general", imageSrc: () => undefined };
});
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));

import i18n, { detectLanguage } from "./i18n";
import App from "./App";
import { useStore, defaultSelection } from "./store";
import Library from "./pages/Library";
import ScanPage from "./pages/ScanPage";
import RestoreDialog from "./pages/RestoreDialog";
import Wizard from "./pages/Wizard";
import GameDetail from "./pages/GameDetail";
import { nextTheme } from "./theme";

const info = { version: "0.1.0", portable: false, firstRun: false, locale: "zh-CN", theme: "auto" as const, configPath: "C:/c.toml", defaultRepoDir: "C:/repo", repoDir: "C:/repo", configError: null, host: "PC", protectLocal: true };

function seed(rows = [row(), row({ id: "steam-1", name: "Cuphead", nameZh: null, state: "up-to-date" }), row({ id: "custom-x", name: "Celeste", nameZh: null, platform: "custom", state: "never-backed-up" })]) {
  const map = Object.fromEntries(rows.map((r) => [r.id, r]));
  useStore.setState({ info, rows: map, order: rows.map((r) => r.id), selected: defaultSelection(rows), stateFilter: "all", platformFilter: "all", query: "", loading: false, toasts: [], undoable: {}, view: { name: "library" } });
}

beforeEach(async () => {
  vi.clearAllMocks();
  await i18n.changeLanguage("zh-CN");
});

describe("基础", () => {
  it("语言检测与主题循环", () => {
    expect(detectLanguage("en", "zh-CN")).toBe("en");
    expect(detectLanguage(null, "en-US")).toBe("en");
    expect(detectLanguage(null, "pt-BR")).toBe("zh-CN");
    expect(nextTheme("auto")).toBe("light");
    expect(nextTheme("dark")).toBe("auto");
  });

  it("首次运行显示向导，否则显示游戏库并扫描", async () => {
    api.appInfo.mockResolvedValueOnce({ ...info, firstRun: true });
    const { unmount } = render(<App />);
    expect(await screen.findByText("选择界面语言")).toBeInTheDocument();
    unmount();

    api.appInfo.mockResolvedValueOnce(info);
    render(<App />);
    expect(await screen.findByRole("heading", { name: "游戏库" })).toBeInTheDocument();
    expect(api.loadLibrary).toHaveBeenCalledWith(true);
  });
});

describe("游戏库", () => {
  it("默认勾选有更新与未备份的游戏，并按状态筛选", async () => {
    seed();
    render(<Library />);
    expect(screen.getByText("已选 2 个")).toBeInTheDocument();
    expect(screen.getByRole("checkbox", { name: "选择 空洞骑士" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "选择 Cuphead" })).not.toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: /^已是最新\s*1$/ }));
    expect(screen.queryByText("空洞骑士")).not.toBeInTheDocument();
    expect(screen.getByText("Cuphead")).toBeInTheDocument();
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索游戏" }), { target: { value: "nothing" } });
    fireEvent.click(screen.getByRole("button", { name: "清除筛选" }));
    expect(screen.getByText("空洞骑士")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^全部\s*3$/ }));
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索游戏" }), { target: { value: "cele" } });
    expect(screen.getByText("Celeste")).toBeInTheDocument();
    expect(screen.queryByText("Cuphead")).not.toBeInTheDocument();
  });

  it("备份确认：统一备注填到所有游戏，逐个提交", async () => {
    seed();
    api.runBackup.mockResolvedValue([
      { gameId: "steam-367520", ok: true, result: { result: "created", snapshot: "b3:1", branch: "main", comment: "周末", summary: row().summary, files: 5, bytes: 1, newBytes: 2048, warnings: [] }, error: null },
      { gameId: "custom-x", ok: false, result: null, error: { kind: "general", message: "路径不存在" } },
    ]);
    api.gameRow.mockImplementation(async (id: string) => row({ id, state: "up-to-date" }));
    render(<Library />);
    fireEvent.click(screen.getByRole("button", { name: /备份所选/ }));
    const dialog = await screen.findByRole("dialog", { name: "备份所选游戏" });
    fireEvent.change(within(dialog).getByLabelText("统一备注"), { target: { value: "周末" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "填到所有游戏" }));
    expect(within(dialog).getByLabelText("「空洞骑士」的备注")).toHaveValue("周末");
    fireEvent.click(within(dialog).getByRole("button", { name: "备份 2 个游戏" }));
    await waitFor(() => expect(api.runBackup).toHaveBeenCalled());
    const [items, force, onDetached] = api.runBackup.mock.calls[0];
    expect(items).toEqual([
      { gameId: "steam-367520", comment: "周末", tag: undefined },
      { gameId: "custom-x", comment: "周末", tag: undefined },
    ]);
    expect(force).toBe(false);
    expect(onDetached).toEqual({ mode: "new" });
    expect(await within(dialog).findByText("备份失败：路径不存在")).toBeInTheDocument();
    expect(within(dialog).getByText("已备份：周末（新增 2.0 KB）")).toBeInTheDocument();
  });

  it("备注超过 500 字时不能提交", async () => {
    seed([row()]);
    render(<Library />);
    fireEvent.click(screen.getByRole("button", { name: /备份所选/ }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("「空洞骑士」的备注"), { target: { value: "长".repeat(501) } });
    expect(within(dialog).getByRole("button", { name: "备份 1 个游戏" })).toBeDisabled();
    expect(within(dialog).getByText("501/500")).toBeInTheDocument();
  });

  it("从旧快照恢复后的游戏可选择分支处理方式", async () => {
    seed([row({ detached: true })]);
    api.runBackup.mockResolvedValue([]);
    render(<Library />);
    fireEvent.click(screen.getByRole("button", { name: /备份所选/ }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByLabelText(/接在当前分支上/));
    fireEvent.click(within(dialog).getByRole("button", { name: "备份 1 个游戏" }));
    await waitFor(() => expect(api.runBackup).toHaveBeenCalled());
    expect(api.runBackup.mock.calls[0][2]).toEqual({ mode: "move" });
  });

  it("空游戏库引导去发现游戏", () => {
    seed([]);
    render(<Library />);
    expect(screen.getByText("还没有游戏")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "发现游戏" }));
    expect(useStore.getState().view).toEqual({ name: "scan" });
  });
});

describe("恢复", () => {
  it("显示覆盖与删除的文件、保护快照与云存档提示", async () => {
    api.planRestore.mockResolvedValue({
      target: "b3:aaaaaaaa",
      files: [
        { path: "0/user1.dat", disk: "C:/x/user1.dat", action: "overwrite", size: 1 },
        { path: "0/new.dat", disk: "C:/x/new.dat", action: "delete", size: 1 },
      ],
      unchanged: 3,
      partial: false,
      running: [],
    });
    api.restore.mockResolvedValue({ plan: {}, protection: "b3:pppppppp", dryRun: false });
    const onDone = vi.fn();
    seed();
    render(<RestoreDialog row={row({ cloudSave: true })} target="b3:aaaaaaaa" paths={[]} onClose={() => {}} onDone={onDone} />);
    expect(await screen.findByText("将覆盖 1 个、新建 0 个、删除 1 个文件，3 个相同")).toBeInTheDocument();
    expect(screen.getByText("user1.dat")).toBeInTheDocument();
    expect(screen.getByText(/恢复前会先自动备份一份保护快照/)).toBeInTheDocument();
    expect(screen.getByText(/保留本地存档/)).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText(/完全恢复/));
    await waitFor(() => expect(api.planRestore).toHaveBeenLastCalledWith("steam-367520", "b3:aaaaaaaa", [], true));
    fireEvent.click(screen.getByRole("button", { name: "恢复" }));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(api.restore).toHaveBeenCalledWith("steam-367520", "b3:aaaaaaaa", [], true, false);
    expect(useStore.getState().undoable["steam-367520"]).toBe("b3:aaaaaaaa");
  });

  it("游戏运行中需要勾选“仍然恢复”", async () => {
    api.planRestore.mockResolvedValue({ target: "b3:a", files: [{ path: "0/a.sav", disk: "C:/a.sav", action: "overwrite", size: 1 }], unchanged: 1, partial: false, running: ["Game.exe"] });
    render(<RestoreDialog row={row({ state: "up-to-date" })} target="b3:a" paths={[]} onClose={() => {}} onDone={() => {}} />);
    expect(await screen.findByText(/游戏正在运行：Game.exe/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "恢复" })).toBeDisabled();
    fireEvent.click(screen.getByLabelText("仍然恢复"));
    expect(screen.getByRole("button", { name: "恢复" })).toBeEnabled();
  });
});

describe("历史", () => {
  it("时间线显示快照、标签与当前位置，选中后显示详情", async () => {
    seed();
    api.history.mockResolvedValue(history());
    api.snapshotFiles.mockResolvedValue([{ path: "0/user1.dat", blob: "b3:x", size: 2048, mtimeNs: 0, tag: "save" }]);
    render(<GameDetail id="steam-367520" />);
    expect((await screen.findAllByText("白宫之前")).length).toBe(2);
    expect(screen.getAllByText("当前").length).toBe(2);
    expect(screen.getAllByText("Boss 前").length).toBeGreaterThan(0);
    expect(screen.getByText("恢复前保护")).toBeInTheDocument();
    expect(screen.getByText("打完螳螂领主")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("tab", { name: /文件/ }));
    expect(await screen.findByText("user1.dat")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("option", { name: /打完螳螂领主/ }));
    expect(await screen.findByText(/第二行/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "白宫" } });
    expect(screen.queryByRole("option", { name: /打完螳螂领主/ })).not.toBeInTheDocument();
  });

  it("恢复后显示撤销条", async () => {
    seed();
    api.history.mockResolvedValue(history());
    api.pendingUndo.mockResolvedValue({ target: "b3:aaaaaaaa", protection: null });
    api.undoRestore.mockResolvedValue({});
    api.gameRow.mockResolvedValue(row());
    render(<GameDetail id="steam-367520" />);
    const btn = await screen.findByRole("button", { name: "撤销本次恢复" });
    api.pendingUndo.mockResolvedValue(null);
    fireEvent.click(btn);
    await waitFor(() => expect(api.undoRestore).toHaveBeenCalledWith("steam-367520"));
    await waitFor(() => expect(screen.queryByRole("button", { name: "撤销本次恢复" })).not.toBeInTheDocument());
  });
});

describe("向导", () => {
  it("逐步配置后保存，汇总标出改动过的项", async () => {
    useStore.setState({ info: { ...info, firstRun: true } });
    api.saveSettings.mockResolvedValue({ ...info, firstRun: false });
    const onDone = vi.fn();
    render(<Wizard onDone={onDone} />);
    fireEvent.click(screen.getByLabelText("English"));
    expect(await screen.findByText("Choose a language")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByLabelText("Repository location")).toHaveValue("C:/repo");
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByLabelText("Epic"));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByLabelText("Saves only"));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(screen.getByText("Steam、GOG")).toHaveClass("changed");
    fireEvent.click(screen.getByRole("button", { name: "Finish" }));
    await waitFor(() => expect(onDone).toHaveBeenCalled());
    const cfg = api.saveSettings.mock.calls[0][0];
    expect(onDone).toHaveBeenCalledWith(cfg);
    expect(cfg.general.language).toBe("en");
    expect(cfg.backup.disabled_platforms).toEqual(["epic"]);
    expect(cfg.backup.include_config_files).toBe(false);
  });
});

describe("发现游戏", () => {
  const found = (over: Partial<import("./types").DiscoveredGame>) => ({
    id: "steam-1",
    name: "Hollow Knight",
    nameZh: "空洞骑士",
    platform: "steam",
    rule: "Hollow Knight",
    state: "new" as const,
    locations: ["C:/Users/p/AppData/LocalLow/Team Cherry/Hollow Knight/*.dat"],
    unconfirmed: 0,
    files: 3,
    bytes: 2048,
    coverThumb: null,
    cloudSave: false,
    playtimeMin: 2520,
    lastPlayed: null,
    ...over,
  });

  it("扫描后由用户挑选加入或忽略，默认不勾选", async () => {
    useStore.setState({ scan: null, scanning: false, toasts: [] });
    api.discoverGames.mockResolvedValue({
      platforms: ["steam"],
      rules: { path: "x", games: 1, userGames: 0 },
      games: [found({}), found({ id: "steam-2", name: "Indie Quest", nameZh: null, rule: null, unconfirmed: 1 }), found({ id: "steam-3", name: "Cuphead", nameZh: null, state: "managed" })],
      withoutSaves: [{ id: "steam-9", name: "Stray", platform: "steam", rule: "Stray" }],
    });
    render(<ScanPage />);
    expect(screen.getByText("找出本机游戏的存档")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /开始扫描/ }));
    expect(await screen.findByText("空洞骑士")).toBeInTheDocument();
    expect(screen.getAllByText("推测，需确认").some((e) => e.tagName === "SPAN")).toBe(true);
    expect(screen.queryByText("Cuphead")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^已在库中\s*1$/ })).toBeInTheDocument();
    expect(screen.getByText("已选 0 个")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /加入游戏库/ })).toBeDisabled();
    expect(screen.getByText("已安装但没找到存档的游戏（1）")).toBeInTheDocument();
    expect(screen.getByText("本机还没有存档")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("checkbox", { name: "选择 空洞骑士" }));
    fireEvent.click(screen.getByRole("button", { name: /加入游戏库/ }));
    await waitFor(() => expect(api.includeGames).toHaveBeenCalledWith(["steam-1"]));
    expect(api.loadLibrary).toHaveBeenCalledWith(true);
    await waitFor(() => expect(screen.queryByText("空洞骑士")).not.toBeInTheDocument());

    fireEvent.click(screen.getByRole("button", { name: "全选" }));
    fireEvent.click(screen.getByRole("button", { name: "忽略" }));
    await waitFor(() => expect(api.ignoreGames).toHaveBeenCalledWith(["steam-2"]));
    expect(await screen.findByText("没有新发现的游戏")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^全部/ }));
    expect(screen.getAllByText("已在库中").length).toBeGreaterThan(1);
  });

  it("筛选后全选、全不选、反选只作用于列出的游戏", async () => {
    useStore.setState({ scan: null, scanning: false, toasts: [] });
    api.discoverGames.mockResolvedValue({
      platforms: ["steam", "epic"],
      rules: { path: "x", games: 1, userGames: 0 },
      games: [found({}), found({ id: "steam-2", name: "Indie Quest", nameZh: null, rule: null }), found({ id: "epic-3", name: "Hades", nameZh: null, platform: "epic" })],
      withoutSaves: [],
    });
    render(<ScanPage />);
    fireEvent.click(screen.getByRole("button", { name: /开始扫描/ }));
    expect(await screen.findByText("空洞骑士")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("combobox", { name: "按平台筛选" }), { target: { value: "steam" } });
    expect(screen.queryByText("Hades")).not.toBeInTheDocument();
    expect(screen.getByText("筛选出 2 个（共 3 个）")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "全选" }));
    expect(screen.getByText("已选 2 个")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox", { name: "选择 Indie Quest" }));
    fireEvent.click(screen.getByRole("button", { name: "反选" }));
    expect(screen.getByRole("checkbox", { name: "选择 Indie Quest" })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: "选择 空洞骑士" })).not.toBeChecked();

    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "indie" } });
    expect(screen.queryByText("空洞骑士")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "全不选" }));
    expect(screen.getByText("已选 0 个")).toBeInTheDocument();
    // 表头复选框：勾选列出的全部游戏
    fireEvent.click(screen.getByRole("checkbox", { name: "勾选或取消当前列出的全部游戏" }));
    expect(screen.getByText("已选 1 个")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "清除筛选" }));
    expect(screen.getByText("Hades")).toBeInTheDocument();
  });
});

