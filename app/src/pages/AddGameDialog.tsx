import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Icon, Modal } from "../components/ui";
import { useStore } from "../store";

type Mode = "saves" | "shots";

/**
 * 手动添加游戏（模拟器、独立游戏等）：名称 + 一个或多个存档目录；
 * 或“只管理截图”：名称（与游戏栏截图的窗口标题对应）+ 可选的截图文件夹，不备份存档。
 */
export default function AddGameDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [mode, setMode] = useState<Mode>("saves");
  const [name, setName] = useState("");
  const [paths, setPaths] = useState<string[]>([]);
  const [shotDirs, setShotDirs] = useState<string[]>([]);
  const [installDir, setInstallDir] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pick = async (directory: boolean) => {
    const r = await open({ directory, multiple: false, title: t("ui.add.pickTitle") });
    if (typeof r === "string" && !paths.includes(r)) setPaths((p) => [...p, r]);
  };

  const pickShotDir = async () => {
    const r = await open({ directory: true, multiple: false, title: t("ui.shots.addDir") });
    if (typeof r === "string" && !shotDirs.includes(r)) setShotDirs((p) => [...p, r]);
  };

  const pickInstall = async () => {
    const r = await open({ directory: true, multiple: false, title: t("ui.add.installDir") });
    if (typeof r === "string") setInstallDir(r);
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const g = mode === "saves" ? await api.addGame(name.trim(), paths, [], installDir) : await api.addShotsGame(null, name.trim(), shotDirs);
      await store.refreshRow(g.id);
      store.toast("success", t("ui.add.added", { name: g.name }));
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = !!name.trim() && (mode === "shots" || paths.length > 0);
  const list = (items: string[], remove: (p: string) => void) => (
    <ul className="file-list">
      {items.map((p) => (
        <li key={p}>
          <Icon name="folder" size={14} />
          <span className="path" title={p}>
            {p}
          </span>
          <button className="btn ghost small" onClick={() => remove(p)} aria-label={t("ui.remove")}>
            <Icon name="close" size={14} />
          </button>
        </li>
      ))}
    </ul>
  );

  return (
    <Modal
      title={t("ui.add.title")}
      subtitle={mode === "saves" ? t("ui.add.subtitle") : t("ui.add.shotsSubtitle")}
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={submit} disabled={busy || !ready}>
            {t("ui.add.confirm")}
          </button>
        </>
      }
    >
      <div className="seg add-mode" role="radiogroup" aria-label={t("ui.add.mode")}>
        <button role="radio" aria-checked={mode === "saves"} onClick={() => setMode("saves")}>
          <Icon name="backup" size={16} /> {t("ui.add.modeSaves")}
        </button>
        <button role="radio" aria-checked={mode === "shots"} onClick={() => setMode("shots")}>
          <Icon name="camera" size={16} /> {t("ui.add.modeShots")}
        </button>
      </div>
      <div className="row">
        <label htmlFor="game-name">{t("ui.add.name")}</label>
        <input id="game-name" className="field" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      {mode === "saves" ? (
        <>
          <div className="row">
            <label>{t("ui.add.paths")}</label>
            <div>
              {list(paths, (p) => setPaths((x) => x.filter((y) => y !== p)))}
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
        </>
      ) : (
        <>
          <div className="row">
            <label>{t("ui.shots.dirs")}</label>
            <div>
              {list(shotDirs, (p) => setShotDirs((x) => x.filter((y) => y !== p)))}
              <div className="inline" style={{ marginTop: 6 }}>
                <button className="btn small" onClick={pickShotDir}>
                  <Icon name="folder" size={14} /> {t("ui.shots.addDir")}
                </button>
                <span className="muted">{t("ui.add.optional")}</span>
              </div>
            </div>
          </div>
          <p className="hint">{t("ui.add.shotsHint")}</p>
        </>
      )}
      {error && <div className="callout error">{error}</div>}
    </Modal>
  );
}
