import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import type { GameRow } from "../types";
import { Modal, displayName } from "./ui";

/**
 * 移出游戏库前确认（U-15）：说明移出后会怎样，并可选同时删除备份的存档（全部快照）与截图，默认都不勾选。
 * 磁盘上的游戏存档与截图原图都不会被删除。存档安全保护开启时不能删除本机备份的快照。
 */
export default function RemoveDialog({ rows, onClose, onDone }: { rows: GameRow[]; onClose: () => void; onDone: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [snapshots, setSnapshots] = useState(false);
  const [shots, setShots] = useState(false);
  const [busy, setBusy] = useState(false);
  const snapCount = rows.reduce((n, r) => n + r.snapshots, 0);
  const shotCount = rows.reduce((n, r) => n + r.shots, 0);
  const protectedOn = store.info?.protectLocal ?? true;
  const names = rows.map((r) => displayName(r, i18n.language));
  const deleting = snapshots || shots;

  const confirm = async () => {
    setBusy(true);
    let snaps = 0;
    let pics = 0;
    let done = 0;
    for (const r of rows) {
      try {
        const rep = await api.removeGame(r.id, { snapshots, shots });
        snaps += rep.snapshots;
        pics += rep.shots;
        done += 1;
      } catch (e) {
        store.toast("error", t("ui.remove.failed", { name: displayName(r, i18n.language), msg: errorMessage(e) }));
      }
    }
    if (done > 0) {
      store.toast("success", t("ui.remove.done", { n: done }) + (deleting ? t("ui.remove.deleted", { snapshots: snaps, shots: pics }) : ""));
    }
    await store.loadLibrary(false);
    setBusy(false);
    onClose();
    if (done > 0) onDone();
  };

  return (
    <Modal
      title={rows.length === 1 ? t("ui.remove.titleOne", { name: names[0] }) : t("ui.remove.titleMany", { n: rows.length })}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className={`btn ${deleting ? "danger" : "primary"}`} onClick={() => void confirm()} disabled={busy}>
            {deleting ? t("ui.remove.confirmDelete") : t("ui.remove.confirm")}
          </button>
        </>
      }
    >
      {rows.length > 1 && <p className="remove-names">{names.join("、")}</p>}
      <ul className="remove-facts">
        <li>{t("ui.remove.factStop")}</li>
        <li>{t("ui.remove.factRescan")}</li>
        <li>{t("ui.remove.factDisk")}</li>
      </ul>
      <div className="remove-options">
        {snapCount > 0 && (
          <label className={`purge-originals${protectedOn ? " disabled" : ""}`}>
            <input type="checkbox" className="check" checked={snapshots} disabled={protectedOn} onChange={(e) => setSnapshots(e.target.checked)} />
            <span>
              <b>{t("ui.remove.snapshots", { n: snapCount })}</b>
              <small>{protectedOn ? t("ui.remove.snapshotsProtected") : snapshots ? t("ui.remove.cannotUndo") : t("ui.remove.snapshotsKeep")}</small>
            </span>
          </label>
        )}
        {shotCount > 0 && (
          <label className="purge-originals">
            <input type="checkbox" className="check" checked={shots} onChange={(e) => setShots(e.target.checked)} />
            <span>
              <b>{t("ui.remove.shots", { n: shotCount })}</b>
              <small>{shots ? t("ui.remove.cannotUndo") : t("ui.remove.shotsKeep")}</small>
            </span>
          </label>
        )}
      </div>
    </Modal>
  );
}
