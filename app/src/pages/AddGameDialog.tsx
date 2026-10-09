import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Icon, Modal } from "../components/ui";
import { useStore } from "../store";

/** 手动添加游戏（模拟器、独立游戏等）：名称 + 一个或多个存档目录。 */
export default function AddGameDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [name, setName] = useState("");
  const [paths, setPaths] = useState<string[]>([]);
  const [installDir, setInstallDir] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pick = async (directory: boolean) => {
    const r = await open({ directory, multiple: false, title: t("ui.add.pickTitle") });
    if (typeof r === "string" && !paths.includes(r)) setPaths((p) => [...p, r]);
  };

  const pickInstall = async () => {
    const r = await open({ directory: true, multiple: false, title: t("ui.add.installDir") });
    if (typeof r === "string") setInstallDir(r);
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const g = await api.addGame(name.trim(), paths, [], installDir);
      await store.refreshRow(g.id);
      store.toast("success", t("ui.add.added", { name: g.name }));
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t("ui.add.title")}
      subtitle={t("ui.add.subtitle")}
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={submit} disabled={busy || !name.trim() || paths.length === 0}>
            {t("ui.add.confirm")}
          </button>
        </>
      }
    >
      <div className="row">
        <label htmlFor="game-name">{t("ui.add.name")}</label>
        <input id="game-name" className="field" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <div className="row">
        <label>{t("ui.add.paths")}</label>
        <div>
          <ul className="file-list">
            {paths.map((p) => (
              <li key={p}>
                <Icon name="folder" size={14} />
                <span className="path" title={p}>
                  {p}
                </span>
                <button className="btn ghost small" onClick={() => setPaths((x) => x.filter((y) => y !== p))} aria-label={t("ui.remove")}>
                  <Icon name="close" size={14} />
                </button>
              </li>
            ))}
          </ul>
          <div className="inline" style={{ marginTop: 6 }}>
            <button className="btn small" onClick={() => pick(true)}>
              {t("ui.add.pickFolder")}
            </button>
            <button className="btn small" onClick={() => pick(false)}>
              {t("ui.add.pickFile")}
            </button>
          </div>
        </div>
      </div>
      <div className="row">
        <label>{t("ui.add.installDir")}</label>
        <div className="inline">
          <span className="muted">{installDir ?? t("ui.add.optional")}</span>
          <button className="btn small" onClick={pickInstall}>
            {t("ui.add.pickFolder")}
          </button>
        </div>
      </div>
      <p className="hint">{t("ui.add.installHint")}</p>
      {error && <div className="callout error">{error}</div>}
    </Modal>
  );
}
