import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage } from "../api";
import { bytes, dateTime, firstLine } from "../lib/format";
import { useStore } from "../store";
import type { BulkDeleteReport, SnapshotNode } from "../types";
import { Icon, Modal } from "./ui";
import { SpaceSources } from "./SpaceSources";

/**
 * 删除多个快照（详情页 Ctrl / Shift 多选）：先检查哪些不能删除、删除后分支与当前存档会怎样，
 * 用户勾选确认后才删除。不能删除的快照跳过。
 */
export function DeleteSnapshotsDialog({ gameId, ids, nodes, onClose, onDone }: { gameId: string; ids: string[]; nodes: SnapshotNode[]; onClose: () => void; onDone: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [plan, setPlan] = useState<BulkDeleteReport | null>(null);
  const [ack, setAck] = useState(false);
  const [ackBranch, setAckBranch] = useState(false);
  const [busy, setBusy] = useState(false);
  const byId = new Map(nodes.map((n) => [n.id, n]));

  useEffect(() => {
    api
      .deleteSnapshots(gameId, ids, true)
      .then(setPlan)
      .catch((e) => {
        store.toast("error", errorMessage(e));
        onClose();
      });
  }, []);

  const run = async () => {
    if (!plan) return;
    setBusy(true);
    try {
      const r = await api.deleteSnapshots(gameId, plan.removed, false);
      store.toast("success", t("ui.bulkDelete.done", { n: r.removed.length, size: bytes(r.bytes) }));
      onDone();
    } catch (e) {
      store.toast("error", errorMessage(e));
      setBusy(false);
    }
  };

  const when = (id: string) => {
    const n = byId.get(id);
    return n ? dateTime(n.createdAt, i18n.language) : id.slice(0, 12);
  };
  const what = (id: string) => firstLine(byId.get(id)?.comment ?? "") || t("ui.history.noComment");
  const none = !!plan && plan.removed.length === 0;
  const gone = plan?.branches.filter((b) => !b.to) ?? [];
  const shortened = plan?.branches.filter((b) => b.to) ?? [];

  return (
    <Modal
      title={t("ui.bulkDelete.title", { n: ids.length })}
      onClose={busy ? () => {} : onClose}
      wide
      footer={
        none ? (
          <button className="btn primary" onClick={onClose}>
            {t("ui.close")}
          </button>
        ) : (
          <>
            <button className="btn ghost" onClick={onClose} disabled={busy}>
              {t("ui.cancel")}
            </button>
            <button className="btn danger" onClick={() => void run()} disabled={!plan || !ack || (gone.length > 0 && !ackBranch) || busy}>
              <Icon name="trash" /> {t("ui.bulkDelete.confirm", { n: plan?.removed.length ?? 0 })}
            </button>
          </>
        )
      }
    >
      {!plan ? (
        <p className="muted">{t("ui.bulkDelete.checking")}</p>
      ) : (
        <div className="bulk-delete">
          <div className="cleanup-figures">
            <div>
              <b className="num">{plan.removed.length}</b>
              <span>{t("ui.bulkDelete.willDelete")}</span>
            </div>
            <div>
              <b className="num">{bytes(plan.bytes)}</b>
              <span>{t("ui.cleanup.freed")}</span>
            </div>
          </div>

          {plan.blocked.length > 0 && (
            <section className="bd-block">
              <h3>{t("ui.bulkDelete.blockedTitle", { n: plan.blocked.length })}</h3>
              {plan.blocked.some((b) => b.reason === "protected") && <p className="hint">{t("ui.bulkDelete.protectedHint")}</p>}
              <ul className="bd-list">
                {plan.blocked.map((b) => (
                  <li key={b.id}>
                    <span className="num">{when(b.id)}</span>
                    <span className="bd-what">{what(b.id)}</span>
                    <span className="bd-why">{t(`ui.bulkDelete.reason.${b.reason}`, { name: b.detail })}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {none ? (
            <p className="callout">{t("ui.bulkDelete.nothing")}</p>
          ) : (
            <>
              <section>
                <h3>{t("ui.bulkDelete.listTitle")}</h3>
                <ul className="bd-list">
                  {[...plan.removed].sort((a, b) => (byId.get(b)?.createdAt ?? "").localeCompare(byId.get(a)?.createdAt ?? "")).map((id) => (
                    <li key={id}>
                      <span className="num">{when(id)}</span>
                      <span className="bd-what">{what(id)}</span>
                    </li>
                  ))}
                </ul>
              </section>
              {plan.branches.length > 0 && (
                <section className={`bd-branches${gone.length > 0 ? " danger" : ""}`} role="note">
                  <h3>
                    <Icon name="branch" size={14} /> {t("ui.bulkDelete.branchTitle", { n: plan.branches.length })}
                  </h3>
                  <ul>
                    {shortened.map((b) => (
                      <li key={b.name}>
                        <b>{t("ui.bulkDelete.branchShort", { name: b.name })}</b>
                        <span>{t("ui.bulkDelete.branchBack", { name: b.name, when: b.toCreatedAt ? dateTime(b.toCreatedAt, i18n.language) : (b.to ?? "").slice(0, 12), what: what(b.to ?? "") })}</span>
                      </li>
                    ))}
                    {gone.map((b) => (
                      <li key={b.name} className="gone">
                        <b>{t("ui.bulkDelete.branchGoneTitle", { name: b.name })}</b>
                        <span>{t("ui.bulkDelete.branchGone", { name: b.name })}</span>
                      </li>
                    ))}
                  </ul>
                  {gone.length > 0 && (
                    <label className="opt">
                      <input type="checkbox" className="check" checked={ackBranch} onChange={(e) => setAckBranch(e.target.checked)} />
                      {t("ui.bulkDelete.ackBranch", { names: gone.map((b) => `「${b.name}」`).join("") })}
                    </label>
                  )}
                </section>
              )}
              <section>
                <h3>{t("ui.bulkDelete.effectsTitle")}</h3>
                <ul className="bd-effects">
                  {plan.rewritten > 0 && <li>{t("ui.bulkDelete.rewire", { n: plan.rewritten })}</li>}
                  {plan.headBranch && <li>{t("ui.history.deleteHeadSwitch", { name: plan.headBranch })}</li>}
                  {plan.currentRemoved && <li className="warn">{t("ui.bulkDelete.current")}</li>}
                  <li>{t("ui.bulkDelete.diskSafe")}</li>
                </ul>
              </section>
              <SpaceSources sources={plan.sources ?? []} total={plan.bytes} />
              <div className="callout error bd-safety" role="note">
                <b>{t("ui.bulkDelete.safetyTitle")}</b>
                <span>{t("ui.bulkDelete.safety")}</span>
                <label className="opt">
                  <input type="checkbox" className="check" checked={ack} onChange={(e) => setAck(e.target.checked)} />
                  {t("ui.bulkDelete.ack", { n: plan.removed.length })}
                </label>
              </div>
            </>
          )}
        </div>
      )}
    </Modal>
  );
}
