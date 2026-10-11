import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { scheduleText } from "../lib/auto";
import { useStore } from "../store";
import type { Config, ShotSchedule, ShotsConfig } from "../types";
import { Menu } from "./ui";

/**
 * 图库中的截图自动备份快捷设置（与游戏库的“自动备份”按钮同一套样式）：
 * 开关截图备份、选择定时收集的频率、立即收集一次。详细时刻在设置页中调整。与设置页“截图备份”是同一项配置。
 */
export default function ShotAutoSwitch({ onSynced }: { onSynced: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [cfg, setCfg] = useState<Config | null>(null);
  const [busy, setBusy] = useState(false);
  const info = store.info;

  useEffect(() => {
    if (!info || info.firstRun) return;
    api
      .getSettings()
      .then(setCfg)
      .catch(() => setCfg(null));
  }, [info]);

  if (!info || info.firstRun || !cfg) return null;
  const sh = cfg.shots;
  const summary = !sh.enabled ? t("ui.shotAuto.off") : sh.schedule === "off" ? t("ui.shotAuto.withBackup") : t("ui.shotAuto.on", { what: scheduleText(t, sh) });

  const apply = async (patch: Partial<ShotsConfig>) => {
    setBusy(true);
    try {
      const next: Config = { ...cfg, shots: { ...sh, ...patch } };
      store.setInfo(await api.saveSettings(next));
      setCfg(next);
      const s = next.shots;
      store.toast("success", t("ui.shotAuto.saved", { what: !s.enabled ? t("ui.shotAuto.off") : s.schedule === "off" ? t("ui.shotAuto.withBackup") : t("ui.shotAuto.on", { what: scheduleText(t, s) }) }));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const syncNow = async () => {
    setBusy(true);
    try {
      const r = await api.syncShots(null);
      const n = r.reduce((x, g) => x + g.added, 0);
      store.toast("success", n > 0 ? t("ui.shots.synced", { n }) : t("ui.shots.syncedNone"));
      onSynced();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const sched = (s: ShotSchedule) => ({
    label: s === "off" ? t("ui.shotAuto.onlyWithBackup") : scheduleText(t, { ...sh, schedule: s }),
    checked: sh.enabled && sh.schedule === s,
    disabled: busy || !sh.enabled,
    onClick: () => void apply({ schedule: s }),
  });

  return (
    <Menu
      className={`btn protect auto-switch ${sh.enabled && sh.schedule !== "off" ? "on" : "off"}`}
      icon="clock"
      label={<span className="bar-label">{summary}</span>}
      title={t("ui.shotAuto.tip")}
      items={[
        { label: t("ui.settings.shotsEnabled"), hint: t("ui.settings.shotsEnabledHint"), checked: sh.enabled, disabled: busy, onClick: () => void apply({ enabled: !sh.enabled }) },
        "divider",
        { label: <b className="menu-head">{t("ui.settings.schedule")}</b>, onClick: () => {}, disabled: true },
        sched("off"),
        sched("daily"),
        sched("weekly"),
        sched("hours"),
        "divider",
        { label: t("ui.shotAuto.syncNow"), onClick: () => void syncNow(), disabled: busy || !sh.enabled },
        { label: t("ui.auto.quick.more"), onClick: () => store.go({ name: "settings" }) },
      ]}
    />
  );
}
