import { invoke } from "@tauri-apps/api/core";

export interface AppInfo {
  version: string;
  portable: boolean;
}

/** Tauri 命令层的封装；前端只通过这里调用后端。 */
export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}
