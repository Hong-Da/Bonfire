import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { useStore } from "../store";
import { Icon, Modal } from "./ui";

/** 进程名比较：不区分大小写，可省略 .exe。 */
const keyOf = (n: string) => n.trim().toLowerCase().replace(/\.exe$/, "");
/** 只留文件名。 */
const fileName = (p: string) => p.trim().split(/[\\/]/).pop() ?? "";

/**
 * 游戏程序（M4-2）：运行检测用的进程名。平台游戏按安装目录检测；
 * 模拟器、手动添加的游戏在这里加上游戏的程序，检测到在运行时阻止恢复、备份前提醒、自动备份等游戏退出后再做。
 */
export function ProcessDialog({ gameId, name, installDir, onClose }: { gameId: string; name: string; installDir: string | null; onClose: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [names, setNames] = useState<string[] | null>(null);
  const [saved, setSaved] = useState<string[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .gameProcesses(gameId)
      .then((n) => {
        setNames(n);
        setSaved(n);
      })
      .catch((e) => {
        store.toast("error", errorMessage(e));
        onClose();
      });
  }, [gameId]);

  const add = (raw: string) => {
    const n = fileName(raw);
    if (!n || !names) return;
    if (!names.some((x) => keyOf(x) === keyOf(n))) setNames([...names, n]);
    setInput("");
  };

  const pick = async () => {
    const p = await open({ filters: [{ name: t("ui.process.programs"), extensions: ["exe"] }], defaultPath: installDir ?? undefined });
    if (typeof p === "string") add(p);
  };

  const save = async () => {
    if (!names) return;
    setBusy(true);
    try {
      const pending = input.trim() ? [...names, fileName(input)] : names;
      await api.setGameProcesses(gameId, pending);
      store.toast("success", t("ui.process.saved", { name }));
      onClose();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const dirty = !!names && (JSON.stringify(names) !== JSON.stringify(saved) || input.trim() !== "");
  return (
    <Modal
      title={t("ui.process.title", { name })}
      subtitle={t("ui.process.intro")}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={() => void save()} disabled={!dirty || busy}>
            {t("ui.save")}
          </button>
        </>
      }
    >
      {names === null ? (
        <p className="muted">{t("ui.loading")}</p>
      ) : (
        <div className="process-body">
          <div className="process-source">
            <b>{t("ui.process.byDir")}</b>
            <span className={installDir ? "path" : "muted"}>{installDir ?? t("ui.process.noDir")}</span>
          </div>
          <div className="process-source">
            <b>{t("ui.process.byName")}</b>
            {names.length === 0 ? (
              <span className="muted">{t("ui.process.none")}</span>
            ) : (
              <ul className="process-list">
                {names.map((n) => (
                  <li key={n}>
                    <code>{n}</code>
                    <button className="btn small ghost" onClick={() => setNames(names.filter((x) => x !== n))} aria-label={t("ui.process.remove", { name: n })}>
                      <Icon name="close" />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="process-add">
            <input
              className="field"
              value={input}
              placeholder="Game.exe"
              aria-label={t("ui.process.inputLabel")}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  add(input);
                }
              }}
            />
            <button className="btn" onClick={() => add(input)} disabled={!input.trim()}>
              {t("ui.process.add")}
            </button>
            <button className="btn" onClick={() => void pick()}>
              <Icon name="folder" /> {t("ui.process.pick")}
            </button>
          </div>
          {!installDir && names.length === 0 && <p className="hint">{t("ui.process.emptyHint")}</p>}
        </div>
      )}
    </Modal>
  );
}
