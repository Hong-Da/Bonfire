import { useState } from "react";
import { useTranslation } from "react-i18next";
import { confirm } from "../lib/confirm";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import { Icon } from "./ui";

/**
 * 游戏库工具栏中的存档安全保护开关：显示当前状态，点击切换。
 * 关闭需要确认；切换后提示结果。与设置页“保护本机的存档备份”是同一项配置。
 */
export default function ProtectSwitch() {
  const { t } = useTranslation();
  const store = useStore();
  const [busy, setBusy] = useState(false);
  const info = store.info;
  if (!info || info.firstRun) return null;
  const on = info.protectLocal;
  const host = info.host;

  const toggle = async () => {
    if (on && !(await confirm(t("ui.settings.protectLocalOff"), { title: t("ui.settings.protectOffTitle"), kind: "warning", icon: "shield", ok: t("ui.settings.protectOffOk") }))) return;
    setBusy(true);
    try {
      const cfg = await api.getSettings();
      cfg.backup.protect_local = !on;
      store.setInfo(await api.saveSettings(cfg));
      store.toast(on ? "info" : "success", on ? t("ui.library.protectDisabled") : t("ui.library.protectEnabled", { host }));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const tip = on ? t("ui.library.protectTipOn", { host }) : t("ui.library.protectTipOff");
  return (
    <button className={`btn protect ${on ? "on" : "off"}`} onClick={() => void toggle()} disabled={busy} title={tip} aria-label={tip} aria-pressed={on}>
      <Icon name="shield" /> <span className="bar-label">{on ? t("ui.library.protectOn") : t("ui.library.protectOff")}</span>
    </button>
  );
}
