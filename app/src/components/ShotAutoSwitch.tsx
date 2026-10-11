import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { scheduleText } from "../lib/auto";
import { useStore } from "../store";
import type { Config, ShotSchedule, ShotsConfig } from "../types";
import type { TFunction } from "i18next";
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
  const sh = { on_change: false, check_minutes: 5, ...cfg.shots };
  const summary = shotSummary(t, sh);

  const apply = async (patch: Partial<ShotsConfig>) => {
    setBusy(true);
    try {
      const next: Config = { ...cfg, shots: { ...sh, ...patch } };
      store.setInfo(await api.saveSettings(next));
      setCfg(next);
      store.toast("success", t("ui.shotAuto.saved", { what: shotSummary(t, { on_change: false, check_minutes: 5, ...next.shots }) }));
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
      className={`btn protect auto-switch ${sh.enabled && (sh.schedule !== "off" || sh.on_change) ? "on" : "off"}`}
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
        { label: t("ui.shotAuto.change"), hint: t("ui.shotAuto.changeHint"), checked: sh.enabled && sh.on_change, disabled: busy || !sh.enabled, onClick: () => void apply({ on_change: !sh.on_change }) },
        { label: <b className="menu-head">{t("ui.shotAuto.checkHead")}</b>, onClick: () => {}, disabled: true },
        ...[...new Set([1, 5, 15, 30, sh.check_minutes])]
          .sort((x, y) => x - y)
          .map((n) => ({
            label: t("ui.auto.quick.checkEvery", { n }),
            checked: sh.check_minutes === n,
            disabled: busy || !sh.enabled || !sh.on_change,
            onClick: () => void apply({ check_minutes: n }),
          })),
        "divider",
        { label: t("ui.shotAuto.syncNow"), onClick: () => void syncNow(), disabled: busy || !sh.enabled },
        { label: t("ui.auto.quick.more"), onClick: () => store.go({ name: "settings", section: "shots" }) },
      ]}
    />
  );
}

/** 按钮上的状态：截图备份关闭 / 只随备份收集 / 定时与变化后收集。 */
function shotSummary(t: TFunction, sh: Required<Pick<ShotsConfig, "on_change" | "check_minutes">> & ShotsConfig): string {
  if (!sh.enabled) return t("ui.shotAuto.off");
  const parts: string[] = [];
  if (sh.schedule !== "off") parts.push(scheduleText(t, sh));
  if (sh.on_change) parts.push(t("ui.auto.quick.changeEvery", { n: sh.check_minutes }));
  return parts.length ? t("ui.shotAuto.on", { what: parts.join(" + ") }) : t("ui.shotAuto.withBackup");
}
