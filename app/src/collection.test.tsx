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
import Library from "./pages/Library";
import GameDetail from "./pages/GameDetail";
import { CollectionExportDialog, CollectionImportDialog } from "./pages/CollectionDialogs";
import Gallery from "./pages/Gallery";
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
    shots: 0,
    newShots: 0,
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
    api.exportPackage.mockResolvedValue({ dest: "D:/out/all.bonfire", bytes: 4096, games: [{ gameId: "steam-367520", gameName: "Hollow Knight", dir: "games/0/", snapshots: 2, shots: 0 }], objects: 9, skipped: [] });
    render(<CollectionExportDialog rows={rows} initial={["steam-367520", "steam-1"]} onClose={() => {}} />);
    const cup = screen.getByText("Cuphead").closest("label")!;
    expect(within(cup).getByRole("checkbox")).toBeDisabled();
    expect(screen.getByText("1 个游戏，2 个快照，0 张截图")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /选择位置并导出/ }));
    // 默认存档和截图都导出。
    await waitFor(() => expect(api.exportPackage).toHaveBeenCalledWith(["steam-367520"], "D:/out/all.bonfire", { saves: true, shots: true }));
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
        { key: "games/0/", target: null, saves: true, shots: false, adoptMain: false },
        { key: "games/1/", target: null, saves: true, shots: false, adoptMain: false },
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
    // 本机主存档是另一条路线时，main 才能删除。
    api.history.mockResolvedValue({ ...history(), primaries: [{ key: "m-1", branch: "other", label: "PC", mine: true }] });
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

describe("篝火包：存档与截图（U-16）", () => {
  it("只导出截图时，没有截图的游戏不能选", async () => {
    const rows = [row({ shots: 3 }), row({ id: "steam-1", name: "Cuphead", nameZh: null, snapshots: 2, shots: 0 })];
    dialog.save.mockResolvedValue("D:/out/p.bonfire");
    api.exportPackage.mockResolvedValue({ dest: "D:/out/p.bonfire", bytes: 10, games: [], objects: 0, skipped: [] });
    render(<CollectionExportDialog rows={rows} initial={["steam-367520", "steam-1"]} onClose={() => {}} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /存档历史/ }));
    expect(within(screen.getByText("Cuphead").closest("label")!).getByRole("checkbox")).toBeDisabled();
    expect(screen.getByText("1 个游戏，0 个快照，3 张截图")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /选择位置并导出/ }));
    await waitFor(() => expect(api.exportPackage).toHaveBeenCalledWith(["steam-367520"], "D:/out/p.bonfire", { saves: false, shots: true }));
  });

  it("导入时可只导入截图，并把新导入的截图写到磁盘", async () => {
    dialog.open.mockResolvedValue("D:/in/p.bonfire");
    api.inspectArchive.mockResolvedValue({
      collection: true,
      createdAt: "2026-10-09T12:00:00+08:00",
      host: "OLD-PC",
      games: [{ key: "games/0/", preview: bundle({ newSnapshots: 2, shots: 4, newShots: 3 }) }],
    });
    api.importArchive.mockResolvedValue([
      { key: "games/0/", gameId: "steam-367520", gameName: "Hollow Knight", report: { importedSnapshots: 0, objects: 0, createdGame: false, importedShots: 3, newShotIds: ["b3:1", "b3:2", "b3:3"], preview: bundle({}) }, error: null },
    ]);
    api.syncShotsToDisk.mockResolvedValue({ written: 3, bytes: 300, exists: 0, noDir: 0, dirs: ["C:/Steam"] });
    render(<CollectionImportDialog onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /历史包 \/ 合集包/ }));
    const saves = await screen.findByRole("checkbox", { name: "导入「Hollow Knight」的存档" });
    fireEvent.click(saves);
    const disk = screen.getByRole("checkbox", { name: /把新导入的截图写到磁盘/ });
    expect(disk).not.toBeChecked();
    fireEvent.click(disk);
    fireEvent.click(screen.getByRole("button", { name: "导入所选（1）" }));
    await waitFor(() => expect(api.importArchive).toHaveBeenCalledWith("D:/in/p.bonfire", [{ key: "games/0/", target: null, saves: false, shots: true, adoptMain: false }]));
    await waitFor(() => expect(api.syncShotsToDisk).toHaveBeenCalledWith("steam-367520", ["b3:1", "b3:2", "b3:3"]));
  });

  it("图库中选中的截图（可跨游戏）导出为篝火包", async () => {
    const shot = (gameId: string, blob: string) => ({ gameId, blob, takenAt: "2026-10-09T21:00:00+08:00", source: "steam" as const, originalPath: `C:/s/${blob}.jpg`, size: 10, addedAt: "2026-10-09T21:00:00+08:00" });
    api.gallery.mockResolvedValue({
      games: [
        { id: "g1", name: "A", nameZh: null, platform: "steam", inLibrary: true, mode: "both" },
        { id: "g2", name: "B", nameZh: null, platform: "steam", inLibrary: true, mode: "both" },
      ],
      shots: [shot("g1", "b3:111111"), shot("g2", "b3:222222")],
      trash: [],
      bytes: 20,
      trashBytes: 0,
      dir: "D:/Repo/screenshots",
    });
    dialog.save.mockResolvedValue("D:/out/s.bonfire");
    api.exportPackage.mockResolvedValue({ dest: "D:/out/s.bonfire", bytes: 20, games: [], objects: 0, skipped: [] });
    render(<Gallery />);
    await screen.findByText(/2 张截图/);
    fireEvent.click(screen.getByRole("checkbox", { name: "全选" }));
    fireEvent.click(screen.getByRole("button", { name: "导出" }));
    await waitFor(() =>
      expect(api.exportPackage).toHaveBeenCalledWith(["g1", "g2"], "D:/out/s.bonfire", { saves: false, shots: true, shotIds: { g1: ["b3:111111"], g2: ["b3:222222"] } }),
    );
  });
});

describe("本机主存档（U-41）", () => {
  it("主存档的路线带标记、不能删除；可设为本机主存档并整理名称，选了恢复时打开恢复确认", async () => {
    seed([row({ offPrimary: true, primary: "main", branch: "main-OLD" })]);
    const h = history();
    const tip = h.refs[0].target;
    api.history.mockResolvedValue({
      ...h,
      refs: [...h.refs, { name: "main-OLD", kind: "branch", target: tip, current: false }],
      nodes: h.nodes.map((n) => (n.id === tip ? { ...n, refs: [...n.refs, "heads/main-OLD"] } : n)),
      primaries: [{ key: "m-1", branch: "main", label: "NEW-PC", mine: true }, { key: "m-2", branch: "main-OLD", label: "OLD-PC", mine: false }],
    });
    api.snapshotFiles.mockResolvedValue([]);
    api.diff.mockResolvedValue([]);
    api.gameRow.mockResolvedValue(row());
    api.planRestore.mockResolvedValue({ target: tip, files: [], unchanged: 1, partial: false, running: [] });
    api.setPrimaryBranch.mockResolvedValue({ branch: "main", renamed: [["main", "main-NEW-PC"], ["main-OLD", "main"]] });
    useStore.setState({ info: { ...(useStore.getState().info ?? {}), host: "NEW-PC" } as never });
    render(<GameDetail id="steam-367520" />);
    await screen.findAllByText("白宫之前");
    // 标记与“回到主存档”。
    expect(document.querySelector(".ref-chip.primary")?.textContent).toContain("main");
    expect(screen.getByRole("button", { name: /回到主存档/ })).toBeInTheDocument();
    // 菜单：main 不能删除；把 main-OLD 设为本机主存档。
    fireEvent.click(screen.getAllByRole("button", { name: "更多操作" }).at(-1)!);
    expect(screen.getByRole("menuitem", { name: "删除分支 main…" })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "设为本机主存档：main-OLD…" }));
    const dlg = await screen.findByRole("dialog", { name: "把「main-OLD」设为本机主存档" });
    expect(dlg).toHaveTextContent("main-NEW-PC");
    fireEvent.click(screen.getByRole("checkbox", { name: /同时恢复到它的最新快照/ }));
    fireEvent.click(screen.getByRole("button", { name: "设为本机主存档" }));
    await waitFor(() => expect(api.setPrimaryBranch).toHaveBeenCalledWith("steam-367520", "main-OLD", true));
    // 选了恢复：打开恢复确认。
    await waitFor(() => expect(api.planRestore).toHaveBeenCalled());
  });

  it("导入时 main 冲突：默认作为新路线，选“我旧电脑的存档”时以它为主存档并恢复", async () => {
    seed([row()]);
    const p = bundle({ newSnapshots: 2, mainConflict: "main-OLD-PC" });
    dialog.open.mockResolvedValue("D:/in/old.bonfire");
    api.inspectArchive.mockResolvedValue({ collection: true, createdAt: p.createdAt, host: "OLD-PC", games: [{ key: "games/0/", preview: p }] });
    api.importArchive.mockResolvedValue([{ key: "games/0/", gameId: "steam-367520", gameName: "Hollow Knight", report: { preview: p, importedSnapshots: 2, objects: 1, createdGame: false }, error: null, adoptedMain: "main" }]);
    api.restore.mockResolvedValue({ plan: {}, protection: "b3:p", dryRun: false });
    render(<CollectionImportDialog onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /历史包 \/ 合集包/ }));
    const group = await screen.findByRole("radiogroup", { name: /包里的 main 和本机的不同/ });
    expect(within(group).getByRole("radio", { name: /作为新路线加入/ })).toBeChecked();
    fireEvent.click(within(group).getByRole("radio", { name: /这是我旧电脑的存档/ }));
    fireEvent.click(screen.getByRole("button", { name: "导入所选（1）" }));
    await waitFor(() => expect(api.importArchive).toHaveBeenCalledWith("D:/in/old.bonfire", [expect.objectContaining({ key: "games/0/", adoptMain: true })]));
    await waitFor(() => expect(api.restore).toHaveBeenCalledWith("steam-367520", "main", [], true, false));
  });
});
