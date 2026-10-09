import i18n from "../i18n";
import { useStore } from "../store";

/** 检查更新并提示结果（游戏库的“检查更新”按钮与托盘菜单共用）。 */
export async function checkAndReport(): Promise<void> {
  const s = useStore.getState();
  if (s.loading) return;
  await s.loadLibrary(false);
  const rows = Object.values(useStore.getState().rows);
  const n = rows.filter((r) => r.state === "has-changes").length;
  s.toast("success", n > 0 ? i18n.t("ui.library.checkedChanges", { n }) : i18n.t("ui.library.checkedNone", { total: rows.length }));
}
