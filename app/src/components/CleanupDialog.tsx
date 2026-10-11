import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { bytes, dateTime, firstLine } from "../lib/format";
import { useStore } from "../store";
import type { CleanupReport } from "../types";
import { Modal } from "./ui";
import { RetentionSafety } from "./RetentionGuide";
import { SpaceSources } from "./SpaceSources";

/** 清理历史（M4-4）：先预览要压掉的快照与能释放的空间，确认后执行。 */
export function CleanupDialog({ onClose }: { onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [plan, setPlan] = useState<CleanupReport | null>(null);
  const [ack, setAck] = useState(false);
  const [done, setDone] = useState<CleanupReport | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .cleanHistory(true)
      .then(setPlan)
      .catch((e) => {
        store.toast("error", errorMessage(e));
        onClose();
      });
  }, []);

  const run = async () => {
    setBusy(true);
    try {
      const r = await api.cleanHistory(false);
      setDone(r);
      if (r.removed > 0) await store.loadLibrary(false);
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const shown = done ?? plan;
  const nothing = !!plan && plan.removed === 0 && plan.gc.bytes === 0 && plan.gc.tmpFiles === 0;
  const footer =
    done || nothing ? (
      <button className="btn primary" onClick={onClose}>
        {t("ui.done")}
      </button>
    ) : (
      <>
        <button className="btn ghost" onClick={onClose} disabled={busy}>
          {t("ui.cancel")}
        </button>
        <button className="btn danger" onClick={() => void run()} disabled={!plan || busy || (plan.removed > 0 && !ack)}>
          {t("ui.cleanup.confirm")}
        </button>
      </>
    );

  return (
    <Modal title={t("ui.cleanup.title")} onClose={busy ? () => {} : onClose} footer={footer}>
      {!shown ? (
        <p className="muted">{t("ui.cleanup.checking")}</p>
      ) : (
        <>
          <div className="cleanup-figures" role="status">
            <div>
              <b className="num">{shown.removed}</b>
              <span>{done ? t("ui.cleanup.removedDone") : t("ui.cleanup.removed")}</span>
            </div>
            <div>
              <b className="num">{bytes(shown.gc.bytes)}</b>
              <span>{done ? t("ui.cleanup.freedDone") : t("ui.cleanup.freed")}</span>
            </div>
          </div>
          {nothing && !done && <p className="muted">{t("ui.cleanup.nothing")}</p>}
          {shown.games.length > 0 && (
            <div className="cleanup-games">
              {shown.games.map((g) => (
                <details key={g.gameId}>
                  <summary>
                    <span>{g.name}</span>
                    <span className="num muted">{t("ui.cleanup.game", { n: g.removed.length, total: g.snapshots })}</span>
                  </summary>
                  <ul className="bd-list">
                    {(g.items ?? []).map((it) => (
                      <li key={it.id}>
                        <span className="num">{dateTime(it.createdAt, i18n.language)}</span>
                        <span className="bd-what">{firstLine(it.comment) || t("ui.history.noComment")}</span>
                        <span className="bd-why">{t(`ui.trigger.${it.trigger}`)}</span>
                      </li>
                    ))}
                  </ul>
                </details>
              ))}
            </div>
          )}
          <SpaceSources sources={shown.gc.sources ?? []} total={shown.gc.bytes} />
          {!done && plan && plan.removed > 0 && (
            <>
              <p className="hint">{t("ui.cleanup.expandHint")}</p>
              <div className="callout error bd-safety" role="note">
                <b>{t("ui.bulkDelete.safetyTitle")}</b>
                <RetentionSafety />
                <label className="opt">
                  <input type="checkbox" className="check" checked={ack} onChange={(e) => setAck(e.target.checked)} />
                  {t("ui.cleanup.ack", { n: plan.removed })}
                </label>
              </div>
            </>
          )}
          {!done && plan && plan.removed === 0 && !nothing && <p className="hint">{t("ui.cleanup.onlySpace")}</p>}
        </>
      )}
    </Modal>
  );
}
