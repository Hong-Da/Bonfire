import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "./api";
import logo from "./assets/logo.svg";
import { Icon, Toasts } from "./components/ui";
import { setLanguage } from "./i18n";
import GameDetail from "./pages/GameDetail";
import Library from "./pages/Library";
import ScanPage from "./pages/ScanPage";
import Settings from "./pages/Settings";
import Wizard from "./pages/Wizard";
import { useStore } from "./store";
import { applyTheme } from "./theme";

export default function App() {
  const { t } = useTranslation();
  const { info, setInfo, view, go, loadLibrary, startScan, toast } = useStore();
  const [ready, setReady] = useState(false);

  useEffect(() => {
    api
      .appInfo()
      .then((i) => {
        setInfo(i);
        // 主题以配置为准；本地存储只用于避免启动时闪烁。首次运行时还没有配置，回到“自动”。
        applyTheme(i.firstRun ? "auto" : i.theme);
        if (!i.firstRun) {
          setLanguage(i.locale);
          void loadLibrary(true);
        }
        if (i.configError) toast("error", t("ui.configError", { msg: i.configError }));
      })
      .catch((e) => toast("error", errorMessage(e)))
      .finally(() => setReady(true));
  }, []);

  if (!ready) return null;

  if (info?.firstRun) {
    return (
      <>
        <Wizard
          onDone={(cfg) => {
            applyTheme(cfg.general.theme);
            if (cfg.general.language) setLanguage(cfg.general.language);
            // 首次运行：进入发现游戏页并开始扫描，由用户选择要管理的游戏。
            go({ name: "scan" });
            void startScan();
          }}
        />
        <Toasts />
      </>
    );
  }

  return (
    <div className="shell">
      <nav className="rail" aria-label={t("ui.nav.label")}>
        <img className="mark" src={logo} alt={t("app.name")} />
        <button aria-current={view.name === "library" || view.name === "game" ? "page" : undefined} onClick={() => go({ name: "library" })}>
          <Icon name="library" size={22} />
          {t("ui.nav.library")}
        </button>
        <button aria-current={view.name === "scan" ? "page" : undefined} onClick={() => go({ name: "scan" })}>
          <Icon name="search" size={22} />
          {t("ui.nav.scan")}
        </button>
        <div className="spacer" />
        <button aria-current={view.name === "settings" ? "page" : undefined} onClick={() => go({ name: "settings" })}>
          <Icon name="settings" size={22} />
          {t("ui.nav.settings")}
        </button>
      </nav>
      <main className="main">
        {view.name === "library" && <Library />}
        {view.name === "scan" && <ScanPage />}
        {view.name === "game" && <GameDetail id={view.id} />}
        {view.name === "settings" && <Settings />}
      </main>
      <Toasts />
    </div>
  );
}
