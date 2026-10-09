import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import logo from "../assets/logo.svg";
import { LANGUAGES, setLanguage } from "../i18n";
import { useStore } from "../store";
import type { Config } from "../types";
import { PLATFORMS } from "./Settings";

export const DEFAULT_CONFIG: Config = {
  general: { theme: "auto" },
  backup: { include_config_files: true, size_warning_gb: 2, disabled_platforms: [] },
  metadata: { online: true, igdb_client_id: "", igdb_client_secret: "", steamgriddb_api_key: "" },
  log: { level: "info", keep_days: 14 },
};

const STEPS = ["language", "repo", "platforms", "config", "info", "summary"] as const;
type Step = (typeof STEPS)[number];

/** 首次运行向导（D4）：语言 → 仓库位置 → 平台开关 → 是否备份配置文件 → 游戏信息联网与 Key → 汇总确认。 */
export default function Wizard({ onDone }: { onDone: (config: Config) => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [step, setStep] = useState<Step>("language");
  const [cfg, setCfg] = useState<Config>(() => ({ ...structuredClone(DEFAULT_CONFIG), general: { theme: "auto", language: i18n.language } }));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const i = STEPS.indexOf(step);
  const update = (f: (c: Config) => void) => {
    const c = structuredClone(cfg);
    f(c);
    setCfg(c);
  };

  const finish = async () => {
    setBusy(true);
    setError(null);
    try {
      const info = await api.saveSettings(cfg);
      store.setInfo(info);
      onDone(cfg);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const repo = cfg.general.repo ?? store.info?.defaultRepoDir ?? "";
  const summary: { label: string; value: string; changed: boolean }[] = [
    { label: t("ui.language"), value: LANGUAGES.find((l) => l.code === cfg.general.language)?.label ?? t("ui.settings.followSystem"), changed: false },
    { label: t("ui.settings.repoPath"), value: repo, changed: !!cfg.general.repo },
    {
      label: t("ui.settings.platforms"),
      value: PLATFORMS.filter((p) => !cfg.backup.disabled_platforms.includes(p)).map((p) => t(`ui.platform.${p}`)).join("、") || t("ui.wizard.none"),
      changed: cfg.backup.disabled_platforms.length > 0,
    },
    { label: t("ui.settings.includeConfig"), value: cfg.backup.include_config_files ? t("ui.yes") : t("ui.no"), changed: !cfg.backup.include_config_files },
    { label: t("ui.settings.online"), value: cfg.metadata.online ? t("ui.yes") : t("ui.no"), changed: !cfg.metadata.online },
  ];

  return (
    <div className="wizard">
      <div className="card">
        <div className="brand">
          <img src={logo} alt="" />
          <h1>{t("app.name")}</h1>
        </div>
        <ol className="steps" aria-label={t("ui.wizard.progress", { n: i + 1, total: STEPS.length })}>
          {STEPS.map((s, k) => (
            <li key={s} className={k <= i ? "done" : ""} />
          ))}
        </ol>

        {step === "language" && (
          <>
            <h2>{t("ui.wizard.languageTitle")}</h2>
            <div className="options">
              {LANGUAGES.map((l) => (
                <label key={l.code} className="option">
                  <input
                    type="radio"
                    name="lang"
                    checked={cfg.general.language === l.code}
                    onChange={() => {
                      setLanguage(l.code);
                      update((c) => (c.general.language = l.code));
                    }}
                  />
                  {l.label}
                </label>
              ))}
            </div>
          </>
        )}

        {step === "repo" && (
          <>
            <h2>{t("ui.wizard.repoTitle")}</h2>
            <p className="muted">{t("ui.wizard.repoBody")}</p>
            <div className="inline" style={{ display: "flex", gap: 8, margin: "14px 0" }}>
              <input className="field" style={{ flex: 1 }} value={repo} onChange={(e) => update((c) => (c.general.repo = e.target.value || undefined))} aria-label={t("ui.settings.repoPath")} />
              <button
                className="btn"
                onClick={async () => {
                  const r = await open({ directory: true, multiple: false, title: t("ui.settings.repo") });
                  if (typeof r === "string") update((c) => (c.general.repo = r));
                }}
              >
                {t("ui.settings.browse")}
              </button>
            </div>
            <p className="hint">{t("ui.settings.repoHint")}</p>
          </>
        )}

        {step === "platforms" && (
          <>
            <h2>{t("ui.wizard.platformsTitle")}</h2>
            <p className="muted">{t("ui.wizard.platformsBody")}</p>
            <div className="options">
              {PLATFORMS.map((p) => (
                <label key={p} className="option">
                  <input
                    type="checkbox"
                    checked={!cfg.backup.disabled_platforms.includes(p)}
                    onChange={(e) =>
                      update((c) => {
                        c.backup.disabled_platforms = e.target.checked ? c.backup.disabled_platforms.filter((x) => x !== p) : [...c.backup.disabled_platforms, p];
                      })
                    }
                  />
                  {t(`ui.platform.${p}`)}
                </label>
              ))}
            </div>
          </>
        )}

        {step === "config" && (
          <>
            <h2>{t("ui.wizard.configTitle")}</h2>
            <div className="options">
              <label className="option">
                <input type="radio" name="cfg" checked={cfg.backup.include_config_files} onChange={() => update((c) => (c.backup.include_config_files = true))} />
                <span>
                  {t("ui.wizard.configYes")}
                  <span className="hint" style={{ display: "block" }}>
                    {t("ui.settings.includeConfigHint")}
                  </span>
                </span>
              </label>
              <label className="option">
                <input type="radio" name="cfg" checked={!cfg.backup.include_config_files} onChange={() => update((c) => (c.backup.include_config_files = false))} />
                {t("ui.wizard.configNo")}
              </label>
            </div>
          </>
        )}

        {step === "info" && (
          <>
            <h2>{t("ui.wizard.infoTitle")}</h2>
            <p className="muted">{t("ui.settings.onlineHint")}</p>
            <label className="option" style={{ margin: "14px 0" }}>
              <input type="checkbox" checked={cfg.metadata.online} onChange={(e) => update((c) => (c.metadata.online = e.target.checked))} />
              {t("ui.settings.online")}
            </label>
            <p className="hint">{t("ui.settings.keysHint")}</p>
          </>
        )}

        {step === "summary" && (
          <>
            <h2>{t("ui.wizard.summaryTitle")}</h2>
            <ul className="summary-list">
              {summary.map((x) => (
                <li key={x.label}>
                  <span>{x.label}</span>
                  <span className={x.changed ? "changed" : ""} title={x.value}>
                    {x.value}
                  </span>
                </li>
              ))}
            </ul>
            <p className="hint">{t("ui.wizard.summaryHint")}</p>
          </>
        )}

        {error && <div className="callout error">{error}</div>}

        <footer>
          <button className="btn ghost" onClick={() => setStep(STEPS[i - 1])} disabled={i === 0 || busy}>
            {t("ui.wizard.back")}
          </button>
          {step === "summary" ? (
            <button className="btn primary" onClick={finish} disabled={busy}>
              {t("ui.wizard.finish")}
            </button>
          ) : (
            <button className="btn primary" onClick={() => setStep(STEPS[i + 1])} disabled={step === "repo" && !repo.trim()}>
              {t("ui.wizard.next")}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
