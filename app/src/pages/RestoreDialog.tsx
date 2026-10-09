import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorKind, errorMessage } from "../api";
import { Modal } from "../components/ui";
import { displayPath, shortId } from "../lib/format";
import { useStore } from "../store";
import type { GameRow, RestorePlan } from "../types";

/**
 * 恢复确认：显示将覆盖、将新建、将删除的文件与保护快照（文档 9.1 交互要点）。
 * 默认只覆盖不删除；“完全恢复”才删除快照中没有的文件。
 */
export default function RestoreDialog({
  row,
  target,
  paths,
  onClose,
  onDone,
}: {
  row: GameRow;
  target: string;
  /** 部分恢复时选中的清单路径。 */
  paths: string[];
  onClose: () => void;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const store = useStore();
  const [full, setFull] = useState(false);
  const [plan, setPlan] = useState<RestorePlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [force, setForce] = useState(false);
  const [busy, setBusy] = useState(false);
  const needsProtection = row.state !== "up-to-date";

  useEffect(() => {
    setPlan(null);
    api
      .planRestore(row.id, target, paths, full)
      .then(setPlan)
      .catch((e) => setError(errorMessage(e)));
  }, [row.id, target, paths, full]);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.restore(row.id, target, paths, full, force);
      store.setUndoable(row.id, target);
      store.toast("success", r.protection ? t("ui.restore.doneProtected", { id: shortId(r.protection) }) : t("ui.restore.done"));
      onDone();
    } catch (e) {
      setError(errorKind(e) === "game-running" ? t("ui.restore.running") : errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const groups: { key: "overwrite" | "create" | "delete"; label: string }[] = [
    { key: "overwrite", label: t("ui.restore.overwrite") },
    { key: "create", label: t("ui.restore.create") },
    { key: "delete", label: t("ui.restore.delete") },
  ];

  return (
    <Modal
      title={t("ui.restore.title", { id: shortId(target) })}
      subtitle={paths.length > 0 ? t("ui.restore.partial", { n: paths.length }) : t("ui.restore.subtitle")}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          {paths.length === 0 && (
            <label style={{ display: "flex", gap: 6, alignItems: "center" }}>
              <input type="checkbox" className="check" checked={full} onChange={(e) => setFull(e.target.checked)} />
              {t("ui.restore.full")}
            </label>
          )}
          <div className="grow" />
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn primary" onClick={run} disabled={busy || !plan || plan.files.length === 0 || (plan.running.length > 0 && !force)}>
            {t("ui.restore.confirm")}
          </button>
        </>
      }
    >
      {needsProtection && <div className="callout">{t("ui.restore.protection")}</div>}
      {row.cloudSave && <div className="callout warn">{t("ui.restore.cloud")}</div>}
      {plan && plan.running.length > 0 && (
        <div className="callout error">
          <p style={{ margin: "0 0 6px" }}>{t("ui.restore.runningList", { p: plan.running.join("、") })}</p>
          <label>
            <input type="checkbox" checked={force} onChange={(e) => setForce(e.target.checked)} /> {t("ui.restore.force")}
          </label>
        </div>
      )}
      {error && <div className="callout error">{error}</div>}
      {!plan && !error && <p className="muted">{t("ui.loading")}</p>}
      {plan && (
        <>
          <p className="num">
            {t("ui.restore.summary", {
              overwrite: plan.files.filter((f) => f.action === "overwrite").length,
              create: plan.files.filter((f) => f.action === "create").length,
              del: plan.files.filter((f) => f.action === "delete").length,
              same: plan.unchanged,
            })}
          </p>
          {groups.map((gr) => {
            const files = plan.files.filter((f) => f.action === gr.key);
            if (files.length === 0) return null;
            return (
              <section key={gr.key}>
                <h3 className="muted" style={{ font: "600 13px var(--font-sign)", margin: "12px 0 4px" }}>
                  {gr.label}
                </h3>
                <ul className="file-list">
                  {files.map((f) => (
                    <li key={f.path} className={f.action}>
                      <span className="mark">{gr.key === "delete" ? "−" : gr.key === "create" ? "+" : "~"}</span>
                      <span className="path" title={f.disk}>
                        {displayPath(f.path)}
                      </span>
                      <span />
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
          {plan.files.length === 0 && <p className="muted">{t("ui.restore.nothing")}</p>}
        </>
      )}
    </Modal>
  );
}
