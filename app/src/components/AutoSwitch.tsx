import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { DEFAULT_AUTO, autoSummary, scheduleText } from "../lib/auto";
import { useStore } from "../store";
import type { AutoBackupConfig, Config, ShotSchedule } from "../types";
import { Menu } from "./ui";

/**
 * 游戏库工具栏中的自动备份快捷开关（与“存档安全保护”按钮并列）：显示当前状态，
 * 菜单中一键选择定时备份的频率、开关“存档变化后自动备份”。详细时刻在设置页中调整。
 */
export default function AutoSwitch() {
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
  const a: AutoBackupConfig = { ...DEFAULT_AUTO, ...cfg.auto_backup };
  const on = a.schedule !== "off" || a.on_change;

  const apply = async (patch: Partial<AutoBackupConfig>) => {
    setBusy(true);
    try {
      const next: Config = { ...cfg, auto_backup: { ...a, ...patch } };
      store.setInfo(await api.saveSettings(next));
      setCfg(next);
      store.toast("success", t("ui.auto.quick.saved", { what: autoSummary(t, next.auto_backup) }));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const sched = (s: ShotSchedule) => ({
    label: scheduleText(t, { ...a, schedule: s }),
    checked: a.schedule === s,
    disabled: busy,
    onClick: () => void apply({ schedule: s }),
  });

  return (
    <Menu
      className={`btn protect auto-switch ${on ? "on" : "off"}`}
      icon="clock"
      label={<span className="bar-label">{autoSummary(t, a)}</span>}
      title={t("ui.auto.quick.tip")}
      align="right"
      items={[
        { label: <b className="menu-head">{t("ui.settings.autoSchedule")}</b>, onClick: () => {}, disabled: true },
        sched("off"),
        sched("daily"),
        sched("weekly"),
        sched("hours"),
        "divider",
        { label: t("ui.settings.autoChange"), hint: t("ui.settings.autoChangeHint"), checked: a.on_change, disabled: busy, onClick: () => void apply({ on_change: !a.on_change }) },
        "divider",
        { label: t("ui.auto.quick.more"), onClick: () => store.go({ name: "settings" }) },
      ]}
    />
  );
}
