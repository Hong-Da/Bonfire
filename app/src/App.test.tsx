import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

vi.mock("./api", () => ({ appInfo: () => Promise.resolve({ version: "0.1.0", portable: false }) }));

import "./i18n";
import i18n, { detectLanguage } from "./i18n";
import App from "./App";
import { nextTheme } from "./theme";

describe("detectLanguage", () => {
  it("优先使用保存的语言", () => expect(detectLanguage("en", "zh-CN")).toBe("en"));
  it("按系统语言匹配", () => expect(detectLanguage(null, "en-US")).toBe("en"));
  it("无法匹配时用中文", () => expect(detectLanguage(null, "pt-BR")).toBe("zh-CN"));
});

describe("theme", () => {
  it("三态循环", () => {
    expect(nextTheme("auto")).toBe("light");
    expect(nextTheme("light")).toBe("dark");
    expect(nextTheme("dark")).toBe("auto");
  });
});

describe("App", () => {
  it("显示名称与版本，主题按钮切换 data-theme", async () => {
    await i18n.changeLanguage("zh-CN");
    render(<App />);
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("篝火");
    expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "主题" }));
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("切换为英文", async () => {
    render(<App />);
    fireEvent.change(screen.getByRole("combobox", { name: "语言" }), { target: { value: "en" } });
    expect(await screen.findByRole("heading", { level: 1 })).toHaveTextContent("Bonfire");
  });
});
