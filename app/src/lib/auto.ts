import type { TFunction } from "i18next";
import type { AutoBackupConfig } from "../types";

/** 自动备份的默认设置（旧配置中没有这一节时使用）。 */
export const DEFAULT_AUTO: AutoBackupConfig = { schedule: "off", at: "21:00", weekday: 7, every_hours: 6, on_change: false, check_minutes: 5, quiet_minutes: 2 };

/** 定时备份的简短说明：“每天 21:00”“每周日 21:00”“每 6 小时”。 */
export function scheduleText(t: TFunction, a: Pick<AutoBackupConfig, "schedule" | "at" | "weekday" | "every_hours">): string {
  switch (a.schedule) {
    case "daily":
      return t("ui.auto.quick.daily", { at: a.at });
    case "weekly":
      return t("ui.auto.quick.weekly", { day: t(`ui.weekday.${a.weekday}`), at: a.at });
    case "hours":
      return t("ui.auto.quick.hours", { n: a.every_hours });
    default:
      return t("ui.auto.quick.off");
  }
}

/** 工具栏按钮上的状态：都关闭时“自动备份：关”，否则列出开启的方式。 */
export function autoSummary(t: TFunction, a: AutoBackupConfig): string {
  const parts: string[] = [];
  if (a.schedule !== "off") parts.push(scheduleText(t, a));
  if (a.on_change) parts.push(t("ui.auto.quick.changeShort"));
  return parts.length ? t("ui.auto.quick.on", { what: parts.join(" + ") }) : t("ui.auto.quick.allOff");
}
