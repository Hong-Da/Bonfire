import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import { Icon, Modal } from "./ui";

/**
 * 第一次关闭窗口时询问：最小化到托盘（推荐，自动备份与定时收集截图继续运行）还是退出篝火。
 * 默认记住选择，之后可以在设置“关闭窗口时”中修改。
 */
export default function CloseDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [tray, setTray] = useState(true);
  const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState(false);

  const confirm = async () => {
    setBusy(true);
    try {
      await api.closeWindow(tray, remember);
      onClose();
    } catch (e) {
      store.toast("error", errorMessage(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("ui.close.title")}
      subtitle={t("ui.close.subtitle")}
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={() => void confirm()} disabled={busy}>
            {tray ? t("ui.close.toTray") : t("ui.close.exit")}
          </button>
        </>
      }
    >
      <div className="close-choices" role="radiogroup" aria-label={t("ui.close.title")}>
        <button role="radio" aria-checked={tray} onClick={() => setTray(true)}>
          <Icon name="clock" size={20} />
          <span>
            <b>{t("ui.close.trayTitle")}</b>
            <small>{t("ui.close.trayBody")}</small>
          </span>
        </button>
        <button role="radio" aria-checked={!tray} onClick={() => setTray(false)}>
          <Icon name="close" size={20} />
          <span>
            <b>{t("ui.close.exitTitle")}</b>
            <small>{t("ui.close.exitBody")}</small>
          </span>
        </button>
      </div>
      <label className="inline remember">
        <input type="checkbox" className="check" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
        {t("ui.close.remember")}
      </label>
    </Modal>
  );
}
