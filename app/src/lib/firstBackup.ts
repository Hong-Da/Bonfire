import type { TFunction } from "i18next";
import { prefGet, prefSet } from "./uiState";
import { api, errorMessage } from "../api";
import { useStore } from "../store";

const PREF_KEY = "bonfire.firstBackup";

/** 加入游戏库后是否立即首次备份（默认开启，记在本机）。 */
export function readFirstBackupPref(): boolean {
  try {
    return prefGet(PREF_KEY) !== "0";
  } catch {
    return true;
  }
}

export function writeFirstBackupPref(on: boolean) {
  try {
    prefSet(PREF_KEY, on ? "1" : "0");
  } catch {
    // 无法保存偏好时只影响本次。
  }
}

/**
 * 加入游戏库后的首次备份（U-14）：逐个备份，备注自动生成（“首次备份：N 个文件”）；
 * 只管理截图的游戏只收集截图。某个游戏失败只提示，不影响其他游戏。备份只读取存档目录。
 */
export async function firstBackup(t: TFunction, ids: string[], names: Record<string, string>): Promise<void> {
  if (ids.length === 0) return;
  const store = useStore.getState();
  store.toast("info", t("ui.first.running", { n: ids.length }));
  try {
    const res = await api.runBackup(
      ids.map((gameId) => ({ gameId })),
      false,
      { mode: "new" },
    );
    const done = res.filter((x) => x.ok && (x.shotsOnly || x.result?.result === "created")).length;
    if (done > 0) store.toast("success", t("ui.first.done", { n: done }));
    for (const f of res.filter((x) => !x.ok)) {
      store.toast("error", t("ui.first.failed", { name: names[f.gameId] ?? f.gameId, msg: f.error?.message ?? "" }));
    }
  } catch (e) {
    store.toast("error", errorMessage(e));
  } finally {
    await Promise.all(ids.map((id) => useStore.getState().refreshRow(id)));
  }
}
