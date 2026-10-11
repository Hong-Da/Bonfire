import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import { Modal, displayName } from "./ui";

/**
 * 把“未识别的截图”归到游戏（U-29）：选游戏库中的游戏，或输入名称新建一个只管理截图的游戏。
 * 之后同名的新截图会自动对应到这个游戏。
 */
export default function AssignDialog({ blobs, onClose, onDone }: { blobs: string[]; onClose: () => void; onDone: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const games = store.order.map((id) => store.rows[id]).filter(Boolean);
  const [target, setTarget] = useState<string>(games[0]?.id ?? "__new");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const isNew = target === "__new";

  const confirm = async () => {
    setBusy(true);
    try {
      let id = target;
      let label = games.find((g) => g.id === target);
      if (isNew) {
        const g = await api.addShotsGame(null, name.trim(), []);
        id = g.id;
        await store.loadLibrary(false);
      }
      const n = await api.assignUnsorted(blobs, id);
      const shown = isNew ? name.trim() : label ? displayName(label, i18n.language) : id;
      store.toast("success", t("ui.unsorted.assigned", { n, name: shown }));
      onDone();
      onClose();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("ui.unsorted.assignTitle", { n: blobs.length })}
      subtitle={t("ui.unsorted.assignHint")}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={() => void confirm()} disabled={busy || (isNew && !name.trim())}>
            {t("ui.unsorted.assign")}
          </button>
        </>
      }
    >
      <div className="row">
        <label htmlFor="assign-game">{t("ui.unsorted.toGame")}</label>
        <select id="assign-game" className="field" value={target} onChange={(e) => setTarget(e.target.value)}>
          {games.map((g) => (
            <option key={g.id} value={g.id}>
              {displayName(g, i18n.language)}
            </option>
          ))}
          <option value="__new">{t("ui.unsorted.newGame")}</option>
        </select>
      </div>
      {isNew && (
        <div className="row">
          <label htmlFor="assign-name">{t("ui.unsorted.newName")}</label>
          <input id="assign-name" className="field" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={t("ui.unsorted.newNameHint")} />
        </div>
      )}
    </Modal>
  );
}
