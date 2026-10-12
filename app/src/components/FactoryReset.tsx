import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { bytes } from "../lib/format";
import { useStore } from "../store";
import type { ResetInfo } from "../types";
import { Icon, Modal } from "./ui";

/**
 * 恢复出厂设置（0.4.2）：列出将清除与保留的内容，可选同时删除备份仓库；
 * 必须输入确认文字（删除仓库时是另一句）才能执行。执行后程序重启进入首次运行向导。
 */
export default function FactoryReset({ encrypted, onClose }: { encrypted: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [deleteRepo, setDeleteRepo] = useState(false);
  const [info, setInfo] = useState<ResetInfo | null>(null);
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setTyped("");
    void api
      .resetPlan(deleteRepo)
      .then(setInfo)
      .catch((e) => store.toast("error", errorMessage(e)));
  }, [deleteRepo]);

  const ok = !!info && typed.trim() === info.phrase;
  const run = async () => {
    if (!info || !ok) return;
    setBusy(true);
    try {
      // 程序会重启；界面的本地存储一起清掉。
      try {
        localStorage.clear();
      } catch {
        /* 忽略 */
      }
      await api.factoryReset(deleteRepo, typed);
    } catch (e) {
      store.toast("error", errorMessage(e));
      setBusy(false);
    }
  };

  const plan = info?.plan;
  return (
    <Modal
      title={t("ui.reset.title")}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t("ui.cancel")}
          </button>
          <button className="btn danger-solid" onClick={() => void run()} disabled={!ok || busy}>
            {deleteRepo ? t("ui.reset.confirmRepo") : t("ui.reset.confirm")}
          </button>
        </>
      }
    >
      <div className="reset">
        <p className="callout error reset-lead">
          <Icon name="info" size={16} /> {t("ui.reset.lead")}
        </p>
        <div className="reset-cols">
          <section>
            <h3>{t("ui.reset.willDelete")}</h3>
            <ul className="reset-list del">
              {(plan?.items ?? []).map((i) => (
                <li key={i.kind}>
                  <span>{t(`ui.reset.kind.${i.kind}`)}</span>
                  <span className="num muted">{bytes(i.bytes)}</span>
                </li>
              ))}
            </ul>
          </section>
          <section>
            <h3>{t("ui.reset.willKeep")}</h3>
            <ul className="reset-list keep">
              {plan && !deleteRepo && <li>{t("ui.reset.keepRepo", { dir: plan.repoDir, size: bytes(plan.repoBytes) })}</li>}
              <li>{t("ui.reset.keepSaves")}</li>
              <li>{t("ui.reset.keepRemote")}</li>
            </ul>
            {!deleteRepo && <p className="hint">{t("ui.reset.keepRepoHint")}</p>}
            {!deleteRepo && encrypted && <p className="hint">{t("ui.reset.passwordNote")}</p>}
          </section>
        </div>
        <label className={`reset-repo${deleteRepo ? " on" : ""}`}>
          <input type="checkbox" className="check" checked={deleteRepo} onChange={(e) => setDeleteRepo(e.target.checked)} disabled={busy} />
          <span>
            <b>{t("ui.reset.deleteRepo")}</b>
            {plan && <small>{t("ui.reset.deleteRepoHint", { dir: plan.repoDir, size: bytes(plan.repoBytes) })}</small>}
          </span>
        </label>
        {info && (
          <label className="reset-type">
            <span>{t("ui.reset.typeToConfirm", { phrase: info.phrase })}</span>
            <input className="field" value={typed} onChange={(e) => setTyped(e.target.value)} disabled={busy} autoComplete="off" spellCheck={false} aria-label={t("ui.reset.typeToConfirm", { phrase: info.phrase })} />
          </label>
        )}
      </div>
    </Modal>
  );
}
