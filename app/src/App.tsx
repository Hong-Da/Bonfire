import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "./api";
import logo from "./assets/logo.svg";
import { Icon, Toasts } from "./components/ui";
import { ConfirmHost } from "./lib/confirm";
import { setLanguage } from "./i18n";
import GameDetail from "./pages/GameDetail";
import Gallery from "./pages/Gallery";
import CloseDialog from "./components/CloseDialog";
import Library from "./pages/Library";
import ScanPage from "./pages/ScanPage";
import Settings from "./pages/Settings";
import Wizard from "./pages/Wizard";
import { useStore } from "./store";
import { applyTheme } from "./theme";
import { checkAndReport } from "./lib/check";

export default function App() {
  const { t } = useTranslation();
  const { info, setInfo, view, go, loadLibrary, startScan, toast } = useStore();
  const [ready, setReady] = useState(false);
  const [closeAsk, setCloseAsk] = useState(false);

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

  // 定时收集截图完成时提示。
  useEffect(() => {
    const off = api.onShotsSynced((n) => {
      if (n > 0) {
        toast("success", t("ui.shots.scheduled", { n }));
        void loadLibrary(false);
      }
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);

  // 托盘菜单“检查更新”：回到游戏库并检查。
  // 自动备份完成：提示结果并刷新游戏库。
  useEffect(() => {
    const off = api.onAutoBackup((r) => {
      const created = r.items.filter((x) => x.snapshot).length;
      const failed = r.items.filter((x) => x.error);
      const shots = r.items.reduce((n, x) => n + x.shots, 0);
      if (created > 0) toast("success", t(r.kind === "scheduled" ? "ui.auto.doneScheduled" : "ui.auto.doneChange", { n: created, names: r.items.filter((x) => x.snapshot).map((x) => x.name).join("、") }));
      if (failed.length > 0) toast("error", t("ui.auto.failed", { names: failed.map((x) => x.name).join("、"), msg: failed[0].error ?? "" }));
      if (created === 0 && failed.length === 0 && shots > 0) toast("success", t("ui.shots.scheduled", { n: shots }));
      void loadLibrary(false);
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);

  // 第一次关闭窗口：询问最小化到托盘还是退出。
  useEffect(() => {
    const off = api.onCloseRequested(() => setCloseAsk(true));
    return () => {
      void off.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const off = api.onTrayCheck(() => {
      if (useStore.getState().info?.firstRun) return;
      go({ name: "library" });
      void checkAndReport();
    });
    return () => {
      void off.then((f) => f());
    };
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
            // 扫描之后检测游戏库（仓库中可能已有加入管理的游戏，例如换电脑后指定了原来的仓库）。
            void startScan().then(() => loadLibrary(false));
          }}
        />
        <ConfirmHost />
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
        <button aria-current={view.name === "gallery" ? "page" : undefined} onClick={() => go({ name: "gallery" })}>
          <Icon name="gallery" size={22} />
          {t("ui.nav.gallery")}
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
        {view.name === "gallery" && <Gallery />}
        {view.name === "scan" && <ScanPage />}
        {view.name === "game" && <GameDetail id={view.id} />}
        {view.name === "settings" && <Settings />}
      </main>
      {closeAsk && <CloseDialog onClose={() => setCloseAsk(false)} />}
      <ConfirmHost />
      <Toasts />
    </div>
  );
}
