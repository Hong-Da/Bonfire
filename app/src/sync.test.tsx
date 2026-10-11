import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { sharedApi as api } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e), imageSrc: () => undefined };
});
const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn(), ask: vi.fn().mockResolvedValue(true) }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);
vi.mock("./lib/confirm", () => ({ confirm: (...a: unknown[]) => dialog.ask(...a), ConfirmHost: () => null }));

import i18n from "./i18n";
import { useStore } from "./store";
import SyncPage, { newTarget } from "./pages/SyncPage";
import type { SyncPlan, TargetStatus } from "./types";

const gitee = (): TargetStatus => ({
  target: { ...newTarget("gitee"), id: "t1", name: "码云", url: "https://gitee.com/u/bak.git", user: "u" },
  hasSecret: true,
  last: null,
  remoteFiles: 0,
});

const plan = (over: Partial<SyncPlan> = {}): SyncPlan => ({
  targetId: "t1",
  upload: 12,
  uploadBytes: 2048,
  delete: 0,
  unchanged: 3,
  excluded: [],
  remoteBytes: 4096,
  limitBytes: 500 * 1024 * 1024,
  level: "ok",
  parts: [["saves", 2048]],
  blocked: null,
  ...over,
});

beforeEach(async () => {
  vi.clearAllMocks();
  api.syncStatus.mockResolvedValue([]);
  api.onSyncProgress.mockResolvedValue(() => {});
  api.onSyncDone.mockResolvedValue(() => {});
  useStore.setState({ toasts: [] });
  await i18n.changeLanguage("zh-CN");
});

describe("同步备份页", () => {
  it("没有目标时引导添加，添加 Gitee 目标并保存令牌", async () => {
    api.syncSave.mockResolvedValue(gitee().target);
    api.syncPlan.mockResolvedValue(plan());
    render(<SyncPage />);
    expect(await screen.findByText("把备份同步到别的地方")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: /添加同步目标/ })[0]);
    fireEvent.click(screen.getByRole("radio", { name: /Gitee/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "仓库地址" }), { target: { value: "https://gitee.com/u/bak.git" } });
    fireEvent.change(screen.getByRole("textbox", { name: "用户名（可留空）" }), { target: { value: "u" } });
    fireEvent.change(screen.getByLabelText("访问令牌"), { target: { value: "tok" } });
    // Gitee 默认不带截图、带篝火设置
    expect(screen.getByRole("checkbox", { name: "带截图" })).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: "篝火设置" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(api.syncSave).toHaveBeenCalled());
    const [t, secret] = api.syncSave.mock.calls[0];
    expect(t).toMatchObject({ kind: "gitee", place: "gitee", url: "https://gitee.com/u/bak.git", user: "u", name: "Gitee" });
    expect(secret).toBe("tok");
    // 新添加的目标询问是否立即同步，确认后打开同步预览。
    await waitFor(() => expect(dialog.ask).toHaveBeenCalledWith(expect.stringContaining("现在同步一次吗"), expect.anything()));
    await waitFor(() => expect(api.syncPlan).toHaveBeenCalledWith("t1"));
  });

  it("超过总大小上限时不能同步，并给出解决办法", async () => {
    api.syncStatus.mockResolvedValue([gitee()]);
    api.syncPlan.mockResolvedValue(plan({ level: "over", blocked: "同步后 600 MB，超过上限 500 MB", excluded: [{ path: "shots/a.png", size: 60 << 20, reason: "超过 50 MB" }] }));
    render(<SyncPage />);
    fireEvent.click(await screen.findByRole("button", { name: "立即同步" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("这次不能同步");
    expect(screen.getByText(/压缩远程仓库，去掉/)).toBeInTheDocument();
    expect(screen.getByText("1 个文件超过单文件上限，不会上传")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /开始同步/ })).toBeDisabled();
  });

  it("预览后开始同步", async () => {
    api.syncStatus.mockResolvedValue([gitee()]);
    api.syncPlan.mockResolvedValue(plan());
    api.syncPush.mockResolvedValue({ id: "t1", run: { at: new Date().toISOString(), uploaded: 12, uploadedBytes: 2048, deleted: 0, excluded: [] }, error: null });
    render(<SyncPage />);
    fireEvent.click(await screen.findByRole("button", { name: "立即同步" }));
    fireEvent.click(await screen.findByRole("button", { name: /开始同步/ }));
    await waitFor(() => expect(api.syncPush).toHaveBeenCalledWith("t1"));
  });
});
