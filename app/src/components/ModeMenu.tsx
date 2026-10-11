import { useTranslation } from "react-i18next";
import { confirm } from "../lib/confirm";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import type { ManageMode } from "../types";
import { Icon, Menu, type IconName } from "./ui";

export const MODES: ManageMode[] = ["both", "saves", "shots"];
export const MODE_ICON: Record<ManageMode, IconName> = { both: "stack", saves: "backup", shots: "camera" };

/** 切换管理方式并刷新游戏库中的这些行；返回是否成功。 */
export function useSetMode() {
  const { t } = useTranslation();
  const store = useStore();
  return async (ids: string[], mode: ManageMode, name?: string): Promise<boolean> => {
    if (ids.length === 0) return false;
    // 批量切换先确认：说明改成什么、影响什么（已有的快照、截图与磁盘上的存档都不受影响）。
    if (ids.length > 1) {
      const rows = useStore.getState().rows;
      const same = ids.filter((id) => rows[id]?.mode === mode).length;
      const text = [
        t("ui.mode.batchConfirm", { n: ids.length, mode: t(`ui.mode.${mode}`) }),
        t(`ui.mode.${mode}Hint`),
        same > 0 ? t("ui.mode.batchSame", { n: same }) : "",
        t("ui.mode.batchSafe"),
      ]
        .filter(Boolean)
        .join("\n\n");
      if (!(await confirm(text, { title: t("ui.mode.batchTitle"), kind: "info" }))) return false;
    }
    try {
      await api.setManageMode(ids, mode);
      await Promise.all(ids.map((id) => store.refreshRow(id)));
      const m = t(`ui.mode.${mode}`);
      store.toast("success", ids.length === 1 && name ? t("ui.mode.doneOne", { name, mode: m }) : t("ui.mode.doneMany", { n: ids.length, mode: m }));
      return true;
    } catch (e) {
      store.toast("error", errorMessage(e));
      return false;
    }
  };
}

/**
 * 管理方式切换（存档和截图 / 只管理存档 / 只管理截图）：游戏库每行的小标签、批量操作与详情页共用。
 * `current` 为空时（批量）不显示勾选。
 */
export default function ModeMenu({
  ids,
  current,
  name,
  variant = "chip",
  label,
  onDone,
}: {
  ids: string[];
  current?: ManageMode;
  name?: string;
  variant?: "chip" | "button";
  label?: string;
  onDone?: () => void;
}) {
  const { t } = useTranslation();
  const setMode = useSetMode();
  const chip = variant === "chip";
  return (
    <Menu
      className={chip ? `mode-chip mode-${current ?? "both"}` : "btn"}
      icon={current ? MODE_ICON[current] : "stack"}
      label={label ?? (current ? t(`ui.mode.${current}`) : t("ui.mode.title"))}
      title={chip ? t("ui.mode.switchHint", { name: name ?? "" }) : undefined}
      align={chip ? "left" : "right"}
      items={MODES.map((m) => ({
        label: (
          <span className="mode-item">
            <b>
              <Icon name={MODE_ICON[m]} size={14} /> {t(`ui.mode.${m}`)}
            </b>
            <small>{t(`ui.mode.${m}Hint`)}</small>
          </span>
        ),
        checked: current === undefined ? undefined : current === m,
        disabled: ids.length === 0,
        onClick: () => void setMode(ids, m, name).then((ok) => ok && onDone?.()),
      }))}
    />
  );
}
