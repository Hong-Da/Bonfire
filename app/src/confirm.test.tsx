import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { sharedApi as api } from "./test-utils";

vi.mock("./api", async () => {
  const { sharedApi } = await import("./test-utils");
  return { api: sharedApi, errorMessage: (e: unknown) => String((e as { message?: string })?.message ?? e) };
});

import i18n from "./i18n";
import { useStore } from "./store";
import { ConfirmHost, confirm } from "./lib/confirm";
import ProtectSwitch from "./components/ProtectSwitch";

const info = { version: "0.1.0", portable: false, firstRun: false, locale: "zh-CN", theme: "auto" as const, configPath: "", defaultRepoDir: "", repoDir: "", configError: null, host: "MY-PC", protectLocal: true };

beforeEach(async () => {
  vi.clearAllMocks();
  useStore.setState({ toasts: [], info });
  await i18n.changeLanguage("zh-CN");
});

describe("应用内确认对话框", () => {
  it("分段显示文字，确定返回 true，取消与 Esc 返回 false", async () => {
    render(<ConfirmHost />);
    let p!: Promise<boolean>;
    act(() => {
      p = confirm("第一段\n\n第二段", { title: "标题" });
    });
    expect(screen.getByRole("dialog", { name: "标题" })).toBeInTheDocument();
    expect(screen.getByText("第一段")).toHaveClass("confirm-lead");
    expect(screen.getByText("第二段")).toHaveClass("confirm-more");
    fireEvent.click(screen.getByRole("button", { name: "确定" }));
    await expect(p).resolves.toBe(true);
    expect(screen.queryByRole("dialog")).toBeNull();

    act(() => {
      p = confirm("再问一次");
    });
    fireEvent.keyDown(window, { key: "Escape" });
    await expect(p).resolves.toBe(false);
  });

  it("关闭存档安全保护：应用内警告对话框，确认后才关闭", async () => {
    const cfg = { general: { theme: "auto" as const, close_to_tray: true }, backup: { include_config_files: true, size_warning_gb: 2, disabled_platforms: [], protect_local: true } };
    api.getSettings.mockResolvedValue(cfg);
    api.saveSettings.mockResolvedValue({ ...info, protectLocal: false });
    render(
      <>
        <ProtectSwitch />
        <ConfirmHost />
      </>,
    );
    fireEvent.click(screen.getByRole("button", { pressed: true }));
    const dlg = await screen.findByRole("dialog", { name: "关闭存档安全保护？" });
    expect(dlg).toHaveTextContent("删除后不能撤销");
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.saveSettings).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { pressed: true }));
    const ok = await screen.findByRole("button", { name: "关闭保护" });
    expect(ok).toHaveClass("danger");
    fireEvent.click(ok);
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ backup: expect.objectContaining({ protect_local: false }) })));
  });
});
