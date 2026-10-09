import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { sharedApi as api, history } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: () => undefined };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import i18n from "./i18n";
import { useStore } from "./store";
import ExportDialog from "./pages/ExportDialog";
import ImportDialog from "./pages/ImportDialog";

beforeEach(async () => {
  vi.clearAllMocks();
  useStore.setState({ toasts: [] });
  await i18n.changeLanguage("zh-CN");
});

describe("导出", () => {
  it("导出选中的快照为 zip", async () => {
    dialog.save.mockResolvedValue("D:/out/hk.zip");
    api.exportFiles.mockResolvedValue({ dest: "D:/out/hk.zip", files: 2, bytes: 100, snapshot: "b3:aaaaaaaa" });
    render(<ExportDialog hist={history()} gameName="空洞骑士" kind="files" snapshot={null} onClose={() => {}} />);
    // 默认导出当前存档；在分支图上点一个快照即改为导出该快照。
    expect(screen.getByRole("radio", { name: "当前磁盘上的存档" })).toBeChecked();
    fireEvent.click(screen.getByRole("option", { name: /打完螳螂领主/ }));
    expect(screen.getByRole("radio", { name: "某个快照" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: /选择位置并导出/ }));
    await waitFor(() => expect(api.exportFiles).toHaveBeenCalledWith("steam-367520", "b3:aaaaaaaa", "D:/out/hk.zip", "zip"));
    expect(await screen.findByRole("button", { name: /在资源管理器中显示/ })).toBeInTheDocument();
  });

  it("历史包：全部分支、一段历史", async () => {
    dialog.save.mockResolvedValue("D:/out/hk.bonfire");
    api.exportBundle.mockResolvedValue({ dest: "D:/out/hk.bonfire", snapshots: 2, objects: 9, bytes: 4096, refs: [], prerequisites: 1 });
    render(<ExportDialog hist={history()} gameName="空洞骑士" kind="bundle" snapshot={null} onClose={() => {}} />);
    expect(screen.getByText("包含 3 个快照")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "一段历史" }));
    // 终点默认是当前所在的快照；点更早的快照作为起点。
    fireEvent.click(screen.getByRole("option", { name: /恢复前的自动保护/ }));
    expect(screen.getByText("包含 2 个快照")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /选择位置并导出/ }));
    await waitFor(() =>
      expect(api.exportBundle).toHaveBeenCalledWith("steam-367520", { kind: "range", from: "b3:bbbbbbbb", to: "b3:cccccccc" }, "D:/out/hk.bonfire", false),
    );
  });
});

describe("导入", () => {
  it("历史包：显示每个分支与本地的关系和新增的快照", async () => {
    dialog.open.mockResolvedValue("D:/in/friend.bonfire");
    api.inspectBundle.mockResolvedValue({
      gameId: "steam-367520",
      gameName: "Hollow Knight",
      localGame: true,
      createdAt: "2026-10-09T12:00:00+08:00",
      host: "FRIEND-PC",
      nodes: [
        { id: "b3:dddddddd", parents: ["b3:cccccccc"], createdAt: "2026-10-09T11:00:00+08:00", trigger: "manual", comment: "朋友打到了深邃巢穴", files: 2, bytes: 100, host: "FRIEND-PC", known: false },
        { id: "b3:cccccccc", parents: ["b3:bbbbbbbb"], createdAt: "2026-10-09T10:00:00+08:00", trigger: "manual", comment: "白宫之前", files: 2, bytes: 100, host: "PC", known: true },
      ],
      newSnapshots: 1,
      missing: [],
      refs: [{ kind: "branch", name: "main", target: "b3:dddddddd", action: "fast-forward", localTarget: "b3:cccccccc", newName: null, forkPoint: null }],
      locationMismatch: false,
    });
    api.importBundle.mockResolvedValue({ importedSnapshots: 1, objects: 4, preview: {} });
    const onDone = vi.fn();
    render(<ImportDialog hist={history()} gameName="空洞骑士" onClose={() => {}} onDone={onDone} />);
    fireEvent.click(screen.getByRole("button", { name: /历史包/ }));
    expect(await screen.findByText("是你这个分支的后续更新，分支将前移")).toBeInTheDocument();
    expect(screen.getByText("2 个快照，其中 1 个是新的")).toBeInTheDocument();
    expect(screen.getByRole("option", { name: /朋友打到了深邃巢穴/ })).toBeInTheDocument();
    expect(screen.getByText("新增")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^导入$/ }));
    await waitFor(() => expect(api.importBundle).toHaveBeenCalledWith("D:/in/friend.bonfire", "steam-367520"));
    expect(onDone).toHaveBeenCalled();
  });

  it("存档文件：默认新分支独立起点，可从分支图上选的快照分出", async () => {
    dialog.open.mockResolvedValue("D:/in/share.zip");
    api.inspectFiles.mockResolvedValue({ files: 2, bytes: 100, folders: [{ folder: "Hollow Knight", ord: 0, files: 2 }], unmapped: [], manifest: null, identical: null });
    api.importFiles.mockResolvedValue({ snapshot: "b3:eeeeeeee", branch: "friend", dryRun: false, preview: {} });
    render(<ImportDialog hist={history()} gameName="空洞骑士" onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /zip 存档/ }));
    expect(await screen.findByRole("radio", { name: /独立起点/ })).toBeChecked();
    fireEvent.click(screen.getByRole("option", { name: /打完螳螂领主/ }));
    expect(screen.getByRole("radio", { name: /从选中的快照分出/ })).toBeChecked();
    fireEvent.change(screen.getByRole("textbox", { name: "新分支名" }), { target: { value: "friend" } });
    fireEvent.click(screen.getByRole("button", { name: /^导入$/ }));
    await waitFor(() =>
      expect(api.importFiles).toHaveBeenCalledWith("steam-367520", "D:/in/share.zip", { kind: "newBranch", name: "friend", parent: "b3:aaaaaaaa" }, null),
    );
  });

  it("存档文件对应不上存档位置时不能导入", async () => {
    dialog.open.mockResolvedValue("D:/in/odd");
    api.inspectFiles.mockResolvedValue({ files: 1, bytes: 1, folders: [{ folder: "whatever", ord: null, files: 1 }], unmapped: ["whatever/a.sav"], manifest: null, identical: null });
    render(<ImportDialog hist={history()} gameName="空洞骑士" onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: /存档文件夹/ }));
    expect(await screen.findByText(/有 1 个文件对应不到存档位置/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^导入$/ })).toBeDisabled();
  });
});
