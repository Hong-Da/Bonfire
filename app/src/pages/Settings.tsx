import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { LANGUAGES, setLanguage } from "../i18n";
import { bytes, dateTime } from "../lib/format";
import { useStore } from "../store";
import { Icon } from "../components/ui";
import { applyTheme } from "../theme";
import type { AutoBackupConfig, AutoStatus, Config, FsckReport, GameRecord, Theme } from "../types";

const DEFAULT_AUTO: AutoBackupConfig = { schedule: "off", at: "21:00", weekday: 7, every_hours: 6, on_change: false, check_minutes: 5, quiet_minutes: 2 };

export const PLATFORMS = ["steam", "epic", "gog"] as const;

/** 设置（文档 9.1）：仓库位置、界面、平台开关、是否备份配置文件、游戏信息来源与 API Key、规则库、维护。 */
export default function Settings() {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [cfg, setCfg] = useState<Config | null>(null);
  const [auto, setAuto] = useState<AutoStatus | null>(null);
  const [saved, setSaved] = useState<Config | null>(null);
  const [rules, setRules] = useState<{ path: string | null; games: number; userGames: number } | null>(null);
  const [fsck, setFsck] = useState<FsckReport | null>(null);
  const [ignored, setIgnored] = useState<GameRecord[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [captures, setCaptures] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings().then((c) => {
      const n = { ...c, auto_backup: c.auto_backup ?? DEFAULT_AUTO };
      setCfg(n);
      setSaved(n);
    });
    api.autoStatus().then(setAuto).catch(() => setAuto(null));
    api.rulesInfo().then(setRules).catch(() => setRules(null));
    api.ignoredGames().then(setIgnored).catch(() => setIgnored([]));
    api.capturesDir().then(setCaptures).catch(() => setCaptures(null));
  }, []);

  if (!cfg) return <div className="empty">{t("ui.loading")}</div>;
  const dirty = JSON.stringify(cfg) !== JSON.stringify(saved);
  const update = (f: (c: Config) => void) => {
    const c = structuredClone(cfg);
    f(c);
    setCfg(c);
  };

  const save = async () => {
    setBusy("save");
    try {
      const repoChanged = (cfg.general.repo ?? "") !== (saved?.general.repo ?? "");
      const info = await api.saveSettings(cfg);
      store.setInfo(info);
      setSaved(cfg);
      if (cfg.general.language) setLanguage(cfg.general.language);
      applyTheme(cfg.general.theme);
      store.toast("success", t("ui.settings.saved"));
      if (repoChanged) await store.loadLibrary(false);
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const run = async (key: string, f: () => Promise<void>) => {
    setBusy(key);
    try {
      await f();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const pickRepo = async () => {
    const r = await open({ directory: true, multiple: false, title: t("ui.settings.repo") });
    if (typeof r === "string") update((c) => (c.general.repo = r));
  };

  return (
    <>
      <div className="toolbar">
        <h1>{t("ui.nav.settings")}</h1>
        <div className="grow" />
        <button className="btn ghost" onClick={() => setCfg(saved)} disabled={!dirty}>
          {t("ui.settings.revert")}
        </button>
        <button className="btn primary" onClick={save} disabled={!dirty || busy === "save"}>
          {t("ui.save")}
        </button>
      </div>
      <div className="page">
        <div className="settings">
          <section>
            <h2>{t("ui.settings.repo")}</h2>
            <div className="row">
              <label>{t("ui.settings.repoPath")}</label>
              <div className="inline">
                <input className="field" style={{ flex: 1 }} value={cfg.general.repo ?? store.info?.defaultRepoDir ?? ""} onChange={(e) => update((c) => (c.general.repo = e.target.value || undefined))} aria-label={t("ui.settings.repoPath")} />
                <button className="btn" onClick={pickRepo}>
                  {t("ui.settings.browse")}
                </button>
              </div>
            </div>
            <p className="hint">{t("ui.settings.repoHint")}</p>
          </section>

          <section>
            <h2>{t("ui.settings.appearance")}</h2>
            <div className="row">
              <label htmlFor="lang">{t("ui.language")}</label>
              <select id="lang" className="field" value={cfg.general.language ?? ""} onChange={(e) => update((c) => (c.general.language = e.target.value || undefined))}>
                <option value="">{t("ui.settings.followSystem")}</option>
                {LANGUAGES.map((l) => (
                  <option key={l.code} value={l.code}>
                    {l.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="row">
              <label>{t("ui.theme")}</label>
              <div className="chips" role="group" aria-label={t("ui.theme")}>
                {(["auto", "light", "dark"] as Theme[]).map((th) => (
                  <button key={th} className="chip" aria-pressed={cfg.general.theme === th} onClick={() => update((c) => (c.general.theme = th))}>
                    {t(`ui.theme.${th}`)}
                  </button>
                ))}
              </div>
            </div>
          </section>

          <section>
            <h2>{t("ui.settings.backup")}</h2>
            <div className="row">
              <label>{t("ui.settings.platforms")}</label>
              <div className="inline">
                {PLATFORMS.map((p) => (
                  <label key={p} style={{ display: "flex", gap: 6, alignItems: "center" }}>
                    <input
                      type="checkbox"
                      className="check"
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
            </div>
            <div className="row">
              <label>{t("ui.settings.includeConfig")}</label>
              <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                <input type="checkbox" className="check" checked={cfg.backup.include_config_files} onChange={(e) => update((c) => (c.backup.include_config_files = e.target.checked))} />
                {t("ui.settings.includeConfigHint")}
              </label>
            </div>
            <div className="row">
              <label htmlFor="warn">{t("ui.settings.sizeWarning")}</label>
              <div className="inline">
                <input id="warn" className="field num" type="number" min={0.1} step={0.5} style={{ width: 100 }} value={cfg.backup.size_warning_gb} onChange={(e) => update((c) => (c.backup.size_warning_gb = Number(e.target.value)))} />
                <span>GB</span>
              </div>
            </div>
          </section>

          <section>
            <h2>{t("ui.settings.auto")}</h2>
            <div className="row">
              <label htmlFor="auto-schedule">{t("ui.settings.autoSchedule")}</label>
              <div className="inline">
                <select id="auto-schedule" className="field" value={cfg.auto_backup.schedule} onChange={(e) => update((c) => (c.auto_backup.schedule = e.target.value as AutoBackupConfig["schedule"]))}>
                  {(["off", "daily", "weekly", "hours"] as const).map((v) => (
                    <option key={v} value={v}>
                      {t(`ui.settings.autoScheduleOpt.${v}`)}
                    </option>
                  ))}
                </select>
                {cfg.auto_backup.schedule === "weekly" && (
                  <select className="field" value={cfg.auto_backup.weekday} onChange={(e) => update((c) => (c.auto_backup.weekday = Number(e.target.value)))} aria-label={t("ui.settings.autoWeekday")}>
                    {[1, 2, 3, 4, 5, 6, 7].map((d) => (
                      <option key={d} value={d}>
                        {t(`ui.weekday.${d}`)}
                      </option>
                    ))}
                  </select>
                )}
                {(cfg.auto_backup.schedule === "daily" || cfg.auto_backup.schedule === "weekly") && (
                  <input className="field num" type="time" value={cfg.auto_backup.at} onChange={(e) => update((c) => (c.auto_backup.at = e.target.value))} aria-label={t("ui.settings.autoAt")} />
                )}
                {cfg.auto_backup.schedule === "hours" && (
                  <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                    <input className="field num" type="number" min={1} max={168} style={{ width: 80 }} value={cfg.auto_backup.every_hours} onChange={(e) => update((c) => (c.auto_backup.every_hours = Number(e.target.value)))} aria-label={t("ui.settings.autoEveryHours")} />
                    {t("ui.settings.autoEveryHoursUnit")}
                  </label>
                )}
              </div>
            </div>
            <div className="row">
              <label>{t("ui.settings.autoChange")}</label>
              <div>
                <label style={{ display: "flex", gap: 6, alignItems: "flex-start" }}>
                  <input type="checkbox" className="check" checked={cfg.auto_backup.on_change} onChange={(e) => update((c) => (c.auto_backup.on_change = e.target.checked))} />
                  {t("ui.settings.autoChangeHint")}
                </label>
                {cfg.auto_backup.on_change && (
                  <div className="inline" style={{ marginTop: 8 }}>
                    <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                      {t("ui.settings.autoCheckEvery")}
                      <input className="field num" type="number" min={1} max={120} style={{ width: 70 }} value={cfg.auto_backup.check_minutes} onChange={(e) => update((c) => (c.auto_backup.check_minutes = Number(e.target.value)))} aria-label={t("ui.settings.autoCheckEvery")} />
                      {t("ui.settings.minutes")}
                    </label>
                    <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                      {t("ui.settings.autoQuiet")}
                      <input className="field num" type="number" min={0} max={60} style={{ width: 70 }} value={cfg.auto_backup.quiet_minutes} onChange={(e) => update((c) => (c.auto_backup.quiet_minutes = Number(e.target.value)))} aria-label={t("ui.settings.autoQuiet")} />
                      {t("ui.settings.minutes")}
                    </label>
                  </div>
                )}
              </div>
            </div>
            <p className="hint">{t("ui.settings.autoHint")}</p>
            {(auto?.lastScheduled || auto?.lastCheck) && (
              <p className="hint num">
                {auto.lastScheduled && t("ui.settings.autoLastScheduled", { when: dateTime(auto.lastScheduled, i18n.language) })}
                {auto.lastScheduled && auto.lastCheck && " · "}
                {auto.lastCheck && t("ui.settings.autoLastCheck", { when: dateTime(auto.lastCheck, i18n.language) })}
              </p>
            )}
          </section>

          <section>
            <h2>{t("ui.settings.shots")}</h2>
            <div className="row">
              <label>{t("ui.settings.shotsEnabled")}</label>
              <label style={{ display: "flex", gap: 6, alignItems: "flex-start" }}>
                <input type="checkbox" className="check" checked={cfg.shots.enabled} onChange={(e) => update((c) => (c.shots.enabled = e.target.checked))} />
                {t("ui.settings.shotsEnabledHint")}
              </label>
            </div>
            <div className="row">
              <label htmlFor="captures">{t("ui.settings.capturesDir")}</label>
              <div className="inline">
                <input
                  id="captures"
                  className="field"
                  style={{ flex: 1 }}
                  value={cfg.shots.captures_dir ?? ""}
                  placeholder={captures ?? ""}
                  onChange={(e) => update((c) => (c.shots.captures_dir = e.target.value || undefined))}
                />
                <button
                  className="btn"
                  onClick={async () => {
                    const r = await open({ directory: true, multiple: false, title: t("ui.settings.capturesDir") });
                    if (typeof r === "string") update((c) => (c.shots.captures_dir = r));
                  }}
                >
                  {t("ui.settings.browse")}
                </button>
              </div>
            </div>
            <p className="hint">{t("ui.settings.capturesDirHint", { dir: captures ?? "—" })}</p>
            <div className="row">
              <label htmlFor="schedule">{t("ui.settings.schedule")}</label>
              <div className="inline">
                <select id="schedule" className="field" value={cfg.shots.schedule} onChange={(e) => update((c) => (c.shots.schedule = e.target.value as typeof c.shots.schedule))}>
                  {(["off", "daily", "weekly", "hours"] as const).map((v) => (
                    <option key={v} value={v}>
                      {t(`ui.settings.schedule.${v}`)}
                    </option>
                  ))}
                </select>
                {cfg.shots.schedule === "weekly" && (
                  <select className="field" value={cfg.shots.weekday} onChange={(e) => update((c) => (c.shots.weekday = Number(e.target.value)))} aria-label={t("ui.settings.schedule.weekly")}>
                    {[1, 2, 3, 4, 5, 6, 7].map((d) => (
                      <option key={d} value={d}>
                        {t(`ui.weekday.${d}`)}
                      </option>
                    ))}
                  </select>
                )}
                {(cfg.shots.schedule === "daily" || cfg.shots.schedule === "weekly") && (
                  <input className="field num" type="time" value={cfg.shots.at} onChange={(e) => update((c) => (c.shots.at = e.target.value))} aria-label={t("ui.settings.at")} />
                )}
                {cfg.shots.schedule === "hours" && (
                  <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                    <input className="field num" type="number" min={1} max={168} style={{ width: 80 }} value={cfg.shots.every_hours} onChange={(e) => update((c) => (c.shots.every_hours = Number(e.target.value)))} />
                    {t("ui.settings.everyHours", { n: cfg.shots.every_hours })}
                  </label>
                )}
              </div>
            </div>
            <p className="hint">{t("ui.settings.scheduleHint")}</p>
            <button
              className="btn"
              disabled={busy === "shots" || !saved?.shots.enabled}
              onClick={() =>
                run("shots", async () => {
                  const r = await api.syncShots(null);
                  const n = r.reduce((s, x) => s + x.added, 0);
                  store.toast("success", n > 0 ? t("ui.shots.synced", { n }) : t("ui.shots.syncedNone"));
                })
              }
            >
              <Icon name="camera" /> {t("ui.settings.syncAll")}
            </button>
          </section>

          <section>
            <h2>{t("ui.settings.safety")}</h2>
            <div className="row">
              <label>{t("ui.settings.protectLocal")}</label>
              <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                <input
                  type="checkbox"
                  className="check"
                  checked={cfg.backup.protect_local}
                  onChange={async (e) => {
                    const on = e.target.checked;
                    // 关闭保护需要确认。
                    if (!on && !(await ask(t("ui.settings.protectLocalOff"), { title: t("ui.settings.safety"), kind: "warning" }))) return;
                    update((c) => (c.backup.protect_local = on));
                  }}
                />
                {t("ui.settings.protectLocalHint", { host: store.info?.host ?? "" })}
              </label>
            </div>
          </section>

          <section>
            <h2>{t("ui.settings.window")}</h2>
            <div className="row">
              <label>{t("ui.settings.closeToTray")}</label>
              <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                <input type="checkbox" className="check" checked={cfg.general.close_to_tray} onChange={(e) => update((c) => (c.general.close_to_tray = e.target.checked))} />
                {t("ui.settings.closeToTrayHint")}
              </label>
            </div>
          </section>

          <section>
            <h2>{t("ui.settings.info")}</h2>
            <div className="row">
              <label>{t("ui.settings.online")}</label>
              <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
                <input type="checkbox" className="check" checked={cfg.metadata.online} onChange={(e) => update((c) => (c.metadata.online = e.target.checked))} />
                {t("ui.settings.onlineHint")}
              </label>
            </div>
            <KeyRow label="IGDB Client ID" value={cfg.metadata.igdb_client_id} onChange={(v) => update((c) => (c.metadata.igdb_client_id = v))} />
            <KeyRow label="IGDB Client Secret" secret value={cfg.metadata.igdb_client_secret} onChange={(v) => update((c) => (c.metadata.igdb_client_secret = v))} />
            <KeyRow label="SteamGridDB API Key" secret value={cfg.metadata.steamgriddb_api_key} onChange={(v) => update((c) => (c.metadata.steamgriddb_api_key = v))} />
            <p className="hint">{t("ui.settings.keysHint")}</p>
          </section>

          <section>
            <h2>{t("ui.settings.rules")}</h2>
            <p className="muted">
              {rules ? (rules.path ? t("ui.settings.rulesInfo", { games: rules.games, user: rules.userGames }) : t("ui.settings.rulesMissing")) : t("ui.loading")}
            </p>
            <button
              className="btn"
              disabled={busy === "rules"}
              onClick={() =>
                run("rules", async () => {
                  const r = await api.updateRules();
                  store.toast("success", r.result === "updated" ? t("ui.settings.rulesUpdated", { games: r.games, size: bytes(r.bytes) }) : t("ui.settings.rulesLatest"));
                  setRules(await api.rulesInfo());
                })
              }
            >
              {t("ui.settings.updateRules")}
            </button>
          </section>

          <section>
            <h2>{t("ui.settings.maintenance")}</h2>
            <div className="inline" style={{ display: "flex", gap: 8 }}>
              <button className="btn" disabled={busy === "fsck"} onClick={() => run("fsck", async () => setFsck(await api.fsck(true)))}>
                {t("ui.settings.fsck")}
              </button>
              <button
                className="btn"
                disabled={busy === "reindex"}
                onClick={() =>
                  run("reindex", async () => {
                    const r = await api.reindex();
                    store.toast("success", t("ui.settings.reindexed", { n: r.snapshots }));
                  })
                }
              >
                {t("ui.settings.reindex")}
              </button>
            </div>
            {fsck && (
              <div className={`callout${fsck.errors.length ? " error" : ""}`} role="status">
                {fsck.errors.length === 0 ? t("ui.settings.fsckOk", { snapshots: fsck.snapshots, files: fsck.blobs }) : t("ui.settings.fsckErrors", { n: fsck.errors.length })}
                {fsck.errors.slice(0, 10).map((e) => (
                  <div key={e}>{e}</div>
                ))}
              </div>
            )}
            {ignored.length > 0 && (
              <>
                <h3 className="muted" style={{ font: "600 13px var(--font-sign)", margin: "16px 0 4px" }}>
                  {t("ui.settings.ignored")}
                </h3>
                <ul className="file-list">
                  {ignored.map((g) => (
                    <li key={g.id}>
                      <span />
                      <span className="path">{g.name}</span>
                      <button
                        className="btn small"
                        onClick={() =>
                          run("enable", async () => {
                            await api.enableGame(g.id);
                            setIgnored(await api.ignoredGames());
                            await store.refreshRow(g.id);
                          })
                        }
                      >
                        {t("ui.settings.unignore")}
                      </button>
                    </li>
                  ))}
                </ul>
              </>
            )}
          </section>

          <section>
            <h2>{t("ui.settings.about")}</h2>
            <p className="muted num">
              {t("ui.settings.version", { v: store.info?.version ?? "" })}
              {store.info?.portable && <span style={{ marginLeft: 8 }}>{t("ui.settings.portable")}</span>}
            </p>
            <button className="btn small" onClick={() => store.info && api.openPath(store.info.configPath)}>
              {t("ui.settings.openConfig")}
            </button>
          </section>
        </div>
      </div>
    </>
  );
}

function KeyRow({ label, value, secret, onChange }: { label: string; value: string; secret?: boolean; onChange: (v: string) => void }) {
  return (
    <div className="row">
      <label>{label}</label>
      <input className="field" type={secret ? "password" : "text"} value={value} onChange={(e) => onChange(e.target.value)} aria-label={label} autoComplete="off" spellCheck={false} />
    </div>
  );
}
