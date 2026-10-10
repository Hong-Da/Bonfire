import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal } from "./ui";

/**
 * 彻底删除截图前确认（U-11）：可选同时删除磁盘上的原图（默认不删）。
 * 原图移到系统回收站，只删除内容与备份一致的；不选时原图保留，但也不会再被收集。
 */
export default function PurgeDialog({ text, onConfirm, onClose }: { text: string; onConfirm: (originals: boolean) => void; onClose: () => void }) {
  const { t } = useTranslation();
  const [originals, setOriginals] = useState(false);
  return (
    <Modal
      title={t("ui.purge.title")}
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("ui.cancel")}
          </button>
          <button
            className="btn danger"
            onClick={() => {
              onClose();
              onConfirm(originals);
            }}
          >
            {originals ? t("ui.purge.confirmBoth") : t("ui.purge.confirm")}
          </button>
        </>
      }
    >
      <p>{text}</p>
      <label className="purge-originals">
        <input type="checkbox" className="check" checked={originals} onChange={(e) => setOriginals(e.target.checked)} />
        <span>
          <b>{t("ui.purge.originals")}</b>
          <small>{originals ? t("ui.purge.originalsOn") : t("ui.purge.originalsOff")}</small>
        </span>
      </label>
    </Modal>
  );
}
