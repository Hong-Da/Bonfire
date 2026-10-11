import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { sharedApi as api, row } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e) };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("./lib/confirm", () => ({ confirm: (...a: unknown[]) => dialog.ask(...a), ConfirmHost: () => null }));

import i18n from "./i18n";
import { useStore } from "./store";
import DownloadImportDialog from "./pages/DownloadImportDialog";
import { CollectionImportDialog } from "./pages/CollectionDialogs";
import type { DownloadPreview } from "./types";

const preview = (over: Partial<DownloadPreview> = {}): DownloadPreview => ({
  kind: "download",
  sourceName: "全收集存档.zip",
  files: [
    { rel: "全收集存档/存档/user1.dat", size: 10, target: "0/user1.dat", ignored: false, replaces: true },
    { rel: "全收集存档/存档/extra.bin", size: 5, target: null, ignored: false, replaces: false },
    { rel: "全收集存档/使用说明.txt", size: 3, target: null, ignored: true, replaces: false },
  ],
  locations: [{ ord: 0, path: "C:/Saves/HK" }],
  mode: "overlay",
  currentFiles: 3,
  existing: ["0/user1.dat", "0/user2.dat", "0/settings.ini"],
  ...over,
});

beforeEach(async () => {
  vi.clearAllMocks();
  useStore.setState({ rows: { "steam-367520": row() }, order: ["steam-367520"], toasts: [] });
  await i18n.changeLanguage("zh-CN");
});

describe("导入下载的存档（U-42）", () => {
  it("加密 zip 先输入密码；自动对应、跳过对不上的；叠加导入后换上", async () => {
    dialog.open.mockResolvedValue("D:/dl/全收集存档.zip");
    api.inspectDownload.mockResolvedValueOnce(preview({ kind: "needs-password", files: [] })).mockResolvedValueOnce(preview());
    api.importDownload.mockResolvedValue({ snapshot: "b3:new", identical: null, branch: "下载-20261011", files: 1 });
    const done = vi.fn();
    render(<DownloadImportDialog gameId="steam-367520" onClose={() => {}} onDone={done} />);
    expect(screen.getByText(/把下载的存档包（zip、7z、rar）选进来/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "选择压缩包…" }));
    expect(await screen.findByText("这个压缩包需要解压密码")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("解压密码"), { target: { value: "pw123" } });
    fireEvent.click(screen.getByRole("button", { name: "解压" }));
    await waitFor(() => expect(api.inspectDownload).toHaveBeenLastCalledWith("steam-367520", "D:/dl/全收集存档.zip", "pw123"));
    expect(await screen.findByText(/找到 2 个存档文件，已自动对应 1 个；另有 1 个说明或图片已忽略/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /下一步/ }));
    expect(screen.getByText(/有 1 个文件没能自动对应/)).toBeInTheDocument();
    expect(screen.getByLabelText("「extra.bin」放到")).toHaveValue("-1");
    // 覆盖现有 / 新增：放到已有文件的位置显示“覆盖现有”，改到别的位置后实时变为“新增文件”。
    // 会覆盖的行可选“覆盖 / 不替换”；新增的标“新增文件”；改位置实时判断。
    const choice = screen.getByRole("radiogroup", { name: "「user1.dat」覆盖还是不替换" });
    expect(within(choice).getByRole("radio", { name: "覆盖" })).toHaveAttribute("aria-checked", "true");
    fireEvent.change(screen.getByLabelText("「extra.bin」放到"), { target: { value: "0" } });
    expect(screen.getAllByText("新增文件")).toHaveLength(1);
    fireEvent.change(screen.getByLabelText("「user1.dat」在存档文件夹中的路径"), { target: { value: "user9.dat" } });
    expect(screen.queryByRole("radiogroup", { name: "「user1.dat」覆盖还是不替换" })).toBeNull();
    expect(screen.getAllByText("新增文件")).toHaveLength(2);
    fireEvent.change(screen.getByLabelText("「user1.dat」在存档文件夹中的路径"), { target: { value: "user1.dat" } });
    fireEvent.change(screen.getByLabelText("「extra.bin」放到"), { target: { value: "-1" } });
    // 选“不替换”后这个文件不导入；“全部覆盖”恢复。
    fireEvent.click(within(screen.getByRole("radiogroup", { name: "「user1.dat」覆盖还是不替换" })).getByRole("radio", { name: "不替换" }));
    expect(screen.getByRole("button", { name: /下一步/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "全部覆盖" }));
    fireEvent.click(screen.getByRole("button", { name: /下一步/ }));
    // 叠加为推荐，默认导入后立即换上。
    expect(screen.getByRole("radio", { name: /叠加到当前存档/ })).toBeChecked();
    expect(screen.getByRole("checkbox", { name: /导入后立即换上这个存档/ })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: /导入并换上/ }));
    await waitFor(() =>
      expect(api.importDownload).toHaveBeenCalledWith("steam-367520", "D:/dl/全收集存档.zip", "pw123", {
        mapping: [["全收集存档/存档/user1.dat", "0/user1.dat"]],
        mode: "overlay",
        branch: expect.stringMatching(/^下载-\d{8}$/),
        comment: "全收集存档",
      }),
    );
    expect(done).toHaveBeenCalledWith("steam-367520", "b3:new");
  });

  it("tar.gz 等与篝火导出的存档：给出操作指引", async () => {
    dialog.open.mockResolvedValue("D:/dl/a.tar.gz");
    api.inspectDownload.mockResolvedValueOnce(preview({ kind: "needs-extract", files: [] }));
    const useBonfire = vi.fn();
    render(<DownloadImportDialog gameId="steam-367520" onClose={() => {}} onDone={() => {}} onUseBonfireImport={useBonfire} />);
    fireEvent.click(screen.getByRole("button", { name: "选择压缩包…" }));
    expect(await screen.findByText("这种压缩包请先解压")).toBeInTheDocument();
    expect(screen.getByText(/篝火可以直接读取 zip、7z、rar/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /下一步/ })).toBeDisabled();
    api.inspectDownload.mockResolvedValueOnce(preview({ kind: "bonfire", files: [] }));
    dialog.open.mockResolvedValue("D:/dl/exported");
    fireEvent.click(screen.getByRole("button", { name: "选择文件夹…" }));
    fireEvent.click(await screen.findByRole("button", { name: "改用“导入存档文件 / 篝火包”" }));
    expect(useBonfire).toHaveBeenCalledWith("D:/dl/exported");
  });

  it("导入失败时提示手工替换：先备份当前存档，打开存档文件夹", async () => {
    dialog.open.mockResolvedValue("D:/dl/broken.zip");
    api.inspectDownload.mockRejectedValue(new Error("读取压缩包失败：invalid Zip archive"));
    api.runBackup.mockResolvedValue([{ gameId: "steam-367520", ok: true, result: { result: "created" }, error: null, shots: 0, shotsOnly: false }]);
    api.openLocation.mockResolvedValue(undefined);
    render(<DownloadImportDialog gameId="steam-367520" onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "选择压缩包…" }));
    expect(await screen.findByText("导入不了？也可以手工替换文件再备份")).toBeInTheDocument();
    expect(screen.getByText(/替换前一定要先备份当前存档/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /立即备份/ }));
    await waitFor(() => expect(api.runBackup).toHaveBeenCalledWith([{ gameId: "steam-367520" }], false, { mode: "new" }));
    expect(await screen.findByRole("button", { name: /已备份/ })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /打开存档文件夹/ }));
    expect(api.openLocation).toHaveBeenCalledWith("steam-367520", 0);
  });

  it("原来的导入中选到不是篝火导出的存档：提示并改用导入下载的存档", async () => {
    dialog.open.mockResolvedValue("D:/dl/hk.zip");
    api.inspectFilesSource.mockRejectedValue(new Error("这些存档文件没有篝火的导出说明，无法判断属于哪个游戏；请在对应游戏的详情页中导入"));
    const use = vi.fn();
    render(<CollectionImportDialog onClose={() => {}} onDone={() => {}} onUseDownload={use} />);
    fireEvent.click(screen.getByRole("button", { name: /存档文件（zip）|zip/ }));
    expect(await screen.findByText("这不是篝火导出的存档")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /改用“导入下载的存档”/ }));
    expect(use).toHaveBeenCalledWith("D:/dl/hk.zip");
  });

  it("篝火导出校验（U-48）：被改过的按普通下载处理并提示；篝火包校验不通过不能导入", async () => {
    dialog.open.mockResolvedValue("D:/dl/Bonfire-空洞骑士.zip");
    api.inspectDownload.mockResolvedValueOnce(preview({ notice: "modified" }));
    render(<DownloadImportDialog gameId="steam-367520" onClose={() => {}} onDone={() => {}} />);
    fireEvent.click(screen.getByRole("button", { name: "选择压缩包…" }));
    expect(await screen.findByText("校验不通过，按普通下载的存档导入")).toBeInTheDocument();
    expect(screen.getByText(/可能被手动修改过，或混入了其他存档文件/)).toBeInTheDocument();
    // 仍可继续导入，放置位置页同样提示。
    fireEvent.click(screen.getByRole("button", { name: /下一步/ }));
    expect(screen.getByText("校验不通过，按普通下载的存档导入")).toBeInTheDocument();

    api.inspectDownload.mockResolvedValueOnce(preview({ kind: "damaged", files: [] }));
    dialog.open.mockResolvedValue("D:/dl/a.bonfire");
    fireEvent.click(screen.getByRole("button", { name: /上一步/ }));
    fireEvent.click(screen.getByRole("button", { name: "选择压缩包…" }));
    expect(await screen.findByText("篝火包校验不通过")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /下一步/ })).toBeDisabled();
  });

  it("原来的导入中选到校验不通过的篝火导出存档：说明原因并改用导入下载的存档", async () => {
    dialog.open.mockResolvedValue("D:/dl/hk.zip");
    api.inspectFilesSource.mockRejectedValue(new Error("这份篝火导出的存档校验不通过：内容与导出时不一致"));
    const use = vi.fn();
    render(<CollectionImportDialog onClose={() => {}} onDone={() => {}} onUseDownload={use} />);
    fireEvent.click(screen.getByRole("button", { name: /存档文件（zip）|zip/ }));
    expect(await screen.findByText("校验不通过，按普通下载的存档导入")).toBeInTheDocument();
    expect(screen.getByText(/不能按篝火导出的导入/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /改用“导入下载的存档”/ }));
    expect(use).toHaveBeenCalledWith("D:/dl/hk.zip");
  });
});
