import { api } from "../api";

/**
 * 界面上的选择（游戏库筛选、图库查看方式与折叠、新加入标记等）的保存位置（U-38）。
 * 启动时从后台读入（数据目录的 ui-state.json），修改时立即交给后台写盘；
 * 同时写一份到浏览器存储，后台不可用时（测试、预览）照常工作。
 */
let cache: Record<string, string> = {};

/** 启动时读入；失败时只用浏览器存储。 */
export async function loadUiState(): Promise<void> {
  try {
    cache = (await api.uiStateGet()) ?? {};
  } catch {
    cache = {};
  }
}

export function prefGet(key: string): string | null {
  if (key in cache) return cache[key];
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function prefSet(key: string, value: string) {
  cache[key] = value;
  try {
    localStorage.setItem(key, value);
  } catch {
    // 浏览器存储不可用时只靠后台保存。
  }
  try {
    void Promise.resolve(api.uiStateSet(key, value)).catch(() => {});
  } catch {
    // 后台不可用时只影响本次。
  }
}

/** 测试用：清空读入的状态。 */
export function resetUiStateForTest() {
  cache = {};
}
