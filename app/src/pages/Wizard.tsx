import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import logo from "../assets/logo.svg";
import { LANGUAGES, setLanguage } from "../i18n";
import { prefSet } from "../lib/uiState";
import { useStore } from "../store";
import type { Config, SyncTarget } from "../types";
import { PLATFORMS } from "./Settings";
import { PLACES, type Place, newTarget, placeOf } from "./SyncPage";

export const DEFAULT_CONFIG: Config = {
  general: { theme: "auto", close_to_tray: true },
  backup: { include_config_files: true, size_warning_gb: 2, disabled_platforms: [], protect_local: true },
  shots: { enabled: true, schedule: "off", at: "21:00", weekday: 7, every_hours: 6 },
  // 首次运行推荐退出游戏后自动备份（决策 WZ-2）。
  auto_backup: { schedule: "off", at: "21:00", weekday: 7, every_hours: 6, on_change: false, check_minutes: 5, quiet_minutes: 2, on_exit: true },
  metadata: { online: true, igdb_client_id: "", igdb_client_secret: "", steamgriddb_api_key: "" },
  log: { level: "info", keep_days: 14 },
};

const FIRST_STEPS = ["language", "remote", "repo", "platforms", "auto", "info", "summary"] as const;
const RERUN_STEPS = ["language", "platforms", "auto", "info", "summary"] as const;
type Step = (typeof FIRST_STEPS)[number];

/** 从远程获取的结果。 */
interface Restored {
  files: number;
  settings: boolean;
  encrypted: boolean;
  target: SyncTarget;
  secret: string;
}

/**
 * 首次运行向导（D4，0.4.0 改版，设计稿 docs/design/wizard-2026-10-11）：
 * 语言 → 从远程获取（可跳过）→ 仓库位置 → 扫描的平台 → 自动备份 → 游戏信息 → 确认。
 * 在设置中重新运行时（`rerun`）按当前设置预填，跳过“从远程获取”，仓库位置只显示不修改，可以取消。
 */
export default function Wizard({ rerun, initial, onDone, onCancel }: { rerun?: boolean; initial?: Config; onDone: (config: Config) => void; onCancel?: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const steps: readonly Step[] = rerun ? RERUN_STEPS : FIRST_STEPS;
  const [step, setStep] = useState<Step>("language");
  const [cfg, setCfg] = useState<Config>(() => (initial ? structuredClone(initial) : { ...structuredClone(DEFAULT_CONFIG), general: { ...DEFAULT_CONFIG.general, language: i18n.language } }));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 从远程获取
  const [fromRemote, setFromRemote] = useState(false);
  const [target, setTarget] = useState<SyncTarget>(() => newTarget("gitee"));
  const [secret, setSecret] = useState("");
  const [dest, setDest] = useState("");
  const [keepSync, setKeepSync] = useState(true);
  const [restored, setRestored] = useState<Restored | null>(null);
  const i = steps.indexOf(step);
  const update = (f: (c: Config) => void) => {
    const c = structuredClone(cfg);
    f(c);
    setCfg(c);
  };

  const repo = cfg.general.repo ?? store.info?.repoDir ?? store.info?.defaultRepoDir ?? "";
  const destDir = dest || store.info?.defaultRepoDir || "";
  const place = placeOf(target);
  const git = target.kind === "github" || target.kind === "gitee";

  const choosePlace = (p: Place) => {
    const d = newTarget(p);
    setTarget({ ...d, url: d.kind === target.kind ? target.url : "", user: target.user });
  };

  const fetchRemote = async () => {
    setBusy(true);
    setError(null);
    try {
      const t0 = { ...target, name: target.name || t(`ui.sync.place.${place}`) };
      const r = await api.wizardRestore(t0, secret || null, destDir);
      setRestored({ files: r.files, settings: !!r.config, encrypted: r.encrypted, target: t0, secret });
      // 用远程的设置预填后面的步骤（保留这里选的语言）；仓库位置为下载到的文件夹。
      const base: Config = r.config ? { ...structuredClone(r.config) } : structuredClone(cfg);
      base.general = { ...base.general, language: cfg.general.language, repo: destDir };
      setCfg(base);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const finish = async () => {
    setBusy(true);
    setError(null);
    try {
      const info = await api.saveSettings(cfg);
      store.setInfo(info);
      // 以后继续同步到获取备份的地方：沿用远程设置中同一地址的目标（保留名称与选项），补上令牌或密码。
      if (restored && keepSync && !info.locked) {
        const same = cfg.sync?.targets.find((x) => x.kind === restored.target.kind && x.url.trim() === restored.target.url.trim());
        try {
          await api.syncSave(same ? { ...same, user: restored.target.user || same.user } : restored.target, restored.secret || null);
        } catch (e) {
          store.toast("error", errorMessage(e));
        }
      }
      if (!rerun) prefSet("tour.pending", "1");
      onDone(cfg);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const autoText = [cfg.auto_backup.on_exit && t("ui.wizard.autoExit"), cfg.auto_backup.on_change && t("ui.wizard.autoChange")].filter(Boolean).join("、") || t("ui.wizard.autoNone");
  const summary: { label: string; value: string; changed: boolean }[] = [
    { label: t("ui.language"), value: LANGUAGES.find((l) => l.code === cfg.general.language)?.label ?? t("ui.settings.followSystem"), changed: false },
    ...(restored ? [{ label: t("ui.wizard.remoteLabel"), value: t("ui.wizard.remoteSummary", { n: restored.files, where: restored.target.url }), changed: true }] : []),
    { label: t("ui.settings.repoPath"), value: repo, changed: !!cfg.general.repo && !rerun },
    {
      label: t("ui.settings.platforms"),
      value: PLATFORMS.filter((p) => !cfg.backup.disabled_platforms.includes(p)).map((p) => t(`ui.platform.${p}`)).join("、") || t("ui.wizard.none"),
      changed: cfg.backup.disabled_platforms.length > 0,
    },
    { label: t("ui.settings.includeConfig"), value: cfg.backup.include_config_files ? t("ui.yes") : t("ui.no"), changed: !cfg.backup.include_config_files },
    { label: t("ui.wizard.autoTitleShort"), value: autoText, changed: false },
    { label: t("ui.wizard.trayLabel"), value: cfg.general.close_to_tray ? t("ui.yes") : t("ui.no"), changed: !cfg.general.close_to_tray },
    { label: t("ui.settings.online"), value: cfg.metadata.online ? t("ui.yes") : t("ui.no"), changed: !cfg.metadata.online },
  ];

  const canNext = !(step === "repo" && !repo.trim()) && !(step === "remote" && fromRemote && !restored);

  return (
    <div className="wizard">
      <div className="card">
        <div className="brand">
          <img src={logo} alt="" />
          <h1>{rerun ? t("ui.wizard.rerunTitle") : t("app.name")}</h1>
        </div>
        <ol className="steps" aria-label={t("ui.wizard.progress", { n: i + 1, total: steps.length })}>
          {steps.map((s, k) => (
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

        {step === "remote" && (
          <>
            <h2>{t("ui.wizard.remoteTitle")}</h2>
            <p className="callout wiz-tip">{t("ui.wizard.remoteTip")}</p>
            <div className="options">
              <label className="option">
                <input type="radio" name="remote" checked={!fromRemote} disabled={!!restored} onChange={() => setFromRemote(false)} />
                <span>
                  {t("ui.wizard.remoteFresh")}
                  <span className="hint" style={{ display: "block" }}>
                    {t("ui.wizard.remoteFreshHint")}
                  </span>
                </span>
              </label>
              <label className="option">
                <input type="radio" name="remote" checked={fromRemote} onChange={() => setFromRemote(true)} />
                <span>
                  {t("ui.wizard.remoteFetch")}
                  <span className="hint" style={{ display: "block" }}>
                    {t("ui.wizard.remoteFetchHint")}
                  </span>
                </span>
              </label>
            </div>
            {fromRemote && !restored && (
              <div className="wiz-remote">
                <label className="wiz-field">
                  <span>{t("ui.sync.type")}</span>
                  <select className="field" value={place} onChange={(e) => choosePlace(e.target.value as Place)}>
                    {PLACES.map((p) => (
                      <option key={p} value={p}>
                        {t(`ui.sync.place.${p}`)}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="wiz-field">
                  <span>{t(`ui.sync.url.${target.kind}`)}</span>
                  <span className="inline">
                    <input className="field" value={target.url} placeholder={t(`ui.sync.urlHint.${place}`)} onChange={(e) => setTarget({ ...target, url: e.target.value })} />
                    {target.kind === "folder" && (
                      <button
                        type="button"
                        className="btn"
                        onClick={async () => {
                          const r = await open({ directory: true, multiple: false, title: t("ui.sync.pickFolder") });
                          if (typeof r === "string") setTarget({ ...target, url: r });
                        }}
                      >
                        {t("ui.settings.browse")}
                      </button>
                    )}
                  </span>
                </label>
                {(target.kind === "gitee" || target.kind === "webdav") && (
                  <label className="wiz-field">
                    <span>{t(target.kind === "gitee" ? "ui.sync.userOptional" : "ui.sync.user")}</span>
                    <input className="field" value={target.user} onChange={(e) => setTarget({ ...target, user: e.target.value })} />
                  </label>
                )}
                {target.kind !== "folder" && (
                  <label className="wiz-field">
                    <span>{t(git ? "ui.sync.token" : "ui.sync.password")}</span>
                    <input className="field" type="password" aria-label={t(git ? "ui.sync.token" : "ui.sync.password")} value={secret} onChange={(e) => setSecret(e.target.value)} />
                  </label>
                )}
                <label className="wiz-field">
                  <span>{t("ui.wizard.remoteDest")}</span>
                  <span className="inline">
                    <input className="field" value={destDir} onChange={(e) => setDest(e.target.value)} aria-label={t("ui.wizard.remoteDest")} />
                    <button
                      type="button"
                      className="btn"
                      onClick={async () => {
                        const r = await open({ directory: true, multiple: false, title: t("ui.wizard.remoteDest") });
                        if (typeof r === "string") setDest(r);
                      }}
                    >
                      {t("ui.settings.browse")}
                    </button>
                  </span>
                  <small className="hint">{t("ui.wizard.remoteDestHint")}</small>
                </label>
                <label className="option plain">
                  <input type="checkbox" checked={keepSync} onChange={(e) => setKeepSync(e.target.checked)} />
                  {t("ui.wizard.keepSync")}
                </label>
                <div className="wiz-actions">
                  <button className="btn primary" disabled={busy || !target.url.trim() || !destDir.trim()} onClick={() => void fetchRemote()}>
                    {busy ? t("ui.wizard.fetching") : t("ui.wizard.fetch")}
                  </button>
                </div>
              </div>
            )}
            {restored && (
              <div className="callout ok-callout" role="status">
                <b>{t("ui.wizard.fetched", { n: restored.files })}</b>
                <p>{restored.settings ? t("ui.wizard.fetchedSettings") : t("ui.wizard.fetchedNoSettings")}</p>
                {restored.encrypted && <p>{t("ui.wizard.fetchedEncrypted")}</p>}
              </div>
            )}
          </>
        )}

        {step === "repo" && (
          <>
            <h2>{t("ui.wizard.repoTitle")}</h2>
            {restored ? (
              <p className="muted">{t("ui.wizard.repoRestored")}</p>
            ) : (
              <p className="muted">{t("ui.wizard.repoBody")}</p>
            )}
            <div className="inline" style={{ display: "flex", gap: 8, margin: "14px 0" }}>
              <input className="field" style={{ flex: 1 }} value={repo} readOnly={!!restored} onChange={(e) => update((c) => (c.general.repo = e.target.value || undefined))} aria-label={t("ui.settings.repoPath")} />
              {!restored && (
                <button
                  className="btn"
                  onClick={async () => {
                    const r = await open({ directory: true, multiple: false, title: t("ui.settings.repo") });
                    if (typeof r === "string") update((c) => (c.general.repo = r));
                  }}
                >
                  {t("ui.settings.browse")}
                </button>
              )}
            </div>
            <p className="hint">{t("ui.settings.repoHint")}</p>
          </>
        )}

        {step === "platforms" && (
          <>
            <h2>{t("ui.wizard.platformsTitle")}</h2>
            <p className="muted">{t("ui.wizard.platformsBody")}</p>
            <div className="options cols">
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
            <label className="option">
              <input type="checkbox" checked={cfg.backup.include_config_files} onChange={(e) => update((c) => (c.backup.include_config_files = e.target.checked))} />
              <span>
                {t("ui.wizard.configYes")}
                <span className="hint" style={{ display: "block" }}>
                  {t("ui.settings.includeConfigHint")}
                </span>
              </span>
            </label>
          </>
        )}

        {step === "auto" && (
          <>
            <h2>{t("ui.wizard.autoTitle")}</h2>
            <p className="muted">{t("ui.wizard.autoBody")}</p>
            <div className="options">
              <label className="option">
                <input type="checkbox" checked={!!cfg.auto_backup.on_exit} onChange={(e) => update((c) => (c.auto_backup.on_exit = e.target.checked))} />
                <span>
                  {t("ui.wizard.autoExit")}
                  <span className="hint" style={{ display: "block" }}>
                    {t("ui.wizard.autoExitHint")}
                  </span>
                </span>
              </label>
              <label className="option">
                <input type="checkbox" checked={cfg.auto_backup.on_change} onChange={(e) => update((c) => (c.auto_backup.on_change = e.target.checked))} />
                <span>
                  {t("ui.wizard.autoChange")}
                  <span className="hint" style={{ display: "block" }}>
                    {t("ui.wizard.autoChangeHint")}
                  </span>
                </span>
              </label>
              <label className="option">
                <input type="checkbox" checked={cfg.general.close_to_tray} onChange={(e) => update((c) => (c.general.close_to_tray = e.target.checked))} />
                <span>
                  {t("ui.wizard.tray")}
                  <span className="hint" style={{ display: "block" }}>
                    {t("ui.wizard.trayHint")}
                  </span>
                </span>
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
            <p className="hint">{rerun ? t("ui.wizard.rerunHint") : t("ui.wizard.summaryHint")}</p>
          </>
        )}

        {error && <div className="callout error">{error}</div>}

        <footer>
          <span className="wiz-left">
            <button className="btn ghost" onClick={() => setStep(steps[i - 1])} disabled={i === 0 || busy}>
              {t("ui.wizard.back")}
            </button>
            {rerun && onCancel && (
              <button className="btn ghost" onClick={onCancel} disabled={busy}>
                {t("ui.cancel")}
              </button>
            )}
          </span>
          {step === "summary" ? (
            <button className="btn primary" onClick={() => void finish()} disabled={busy}>
              {t("ui.wizard.finish")}
            </button>
          ) : step === "remote" && fromRemote && !restored ? (
            <button
              className="btn ghost"
              onClick={() => {
                setFromRemote(false);
                setError(null);
                setStep(steps[i + 1]);
              }}
              disabled={busy}
            >
              {t("ui.wizard.skip")}
            </button>
          ) : (
            <button className="btn primary" onClick={() => setStep(steps[i + 1])} disabled={!canNext}>
              {t("ui.wizard.next")}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
