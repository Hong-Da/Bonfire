import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import logo from "./assets/logo.svg";
import { LANGUAGES, setLanguage } from "./i18n";
import { applyTheme, loadTheme, nextTheme, type Theme } from "./theme";
import { appInfo } from "./api";

export default function App() {
  const { t, i18n } = useTranslation();
  const [theme, setTheme] = useState<Theme>(loadTheme);
  const [version, setVersion] = useState<string>("");

  useEffect(() => applyTheme(theme), [theme]);
  useEffect(() => {
    appInfo().then((info) => setVersion(info.version)).catch(() => setVersion(""));
  }, []);

  return (
    <div className="shell">
      <header className="topbar">
        <img src={logo} alt="" />
        <h1>{t("app.name")}</h1>
        <div className="spacer" />
        <select aria-label={t("ui.language")} value={i18n.language} onChange={(e) => setLanguage(e.target.value)}>
          {LANGUAGES.map((l) => (
            <option key={l.code} value={l.code}>{l.label}</option>
          ))}
        </select>
        <button type="button" onClick={() => setTheme(nextTheme(theme))} aria-label={t("ui.theme")}>
          {t("ui.theme")}：{t(`ui.theme.${theme}`)}
        </button>
      </header>
      <main className="empty">
        <img src={logo} alt={t("app.name")} />
        <h2>{t("app.tagline")}</h2>
        <p>{t("ui.placeholder")}</p>
        {version && <span className="version">v{version}</span>}
      </main>
    </div>
  );
}
