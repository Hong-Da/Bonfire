import i18n from "i18next";
import { initReactI18next } from "react-i18next";
// 与 CLI 共用同一份文案（crates/bonfire-app/locales）。
import zhCN from "../../crates/bonfire-app/locales/zh-CN.json";
import en from "../../crates/bonfire-app/locales/en.json";

export const LANGUAGES = [
  { code: "zh-CN", label: "中文" },
  { code: "en", label: "English" },
] as const;

const STORAGE_KEY = "bonfire.lang";

/** 保存的选择优先；否则按系统语言匹配；无法匹配时用中文。 */
export function detectLanguage(saved: string | null, navigatorLang: string | undefined): string {
  if (saved === "zh-CN" || saved === "en") return saved;
  const primary = (navigatorLang ?? "").toLowerCase().split(/[-_]/)[0];
  return primary === "en" ? "en" : "zh-CN";
}

function readSaved(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

export function setLanguage(code: string) {
  try {
    localStorage.setItem(STORAGE_KEY, code);
  } catch {
    /* 存储不可用时只在本次生效 */
  }
  void i18n.changeLanguage(code);
  document.documentElement.lang = code;
}

void i18n.use(initReactI18next).init({
  resources: { "zh-CN": { translation: zhCN }, en: { translation: en } },
  lng: detectLanguage(readSaved(), navigator.language),
  fallbackLng: "zh-CN",
  keySeparator: false,
  interpolation: { escapeValue: false },
});

export default i18n;
