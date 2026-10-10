import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Icon, Modal } from "../components/ui";
import { bytes } from "../lib/format";
import { useStore } from "../store";
import type { ShotDiff } from "../types";

/**
 * 截图与磁盘对比：备份中的截图和磁盘上的截图文件夹（Steam、游戏栏、添加的文件夹）各有哪些。
 * - 磁盘上的新截图 → 收集；
 * - 在回收站中或已彻底删除、磁盘上原图还在的 → 不会再收集，可以重新收集；
 * - 备份中有、磁盘上已经没有的 → 写回原位置或写到指定文件夹（只写入不存在的文件，不覆盖）。
 */
export default function ShotDiffDialog({ gameId, gameName, onClose, onChanged }: { gameId: string; gameName: string; onClose: () => void; onChanged: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const [diff, setDiff] = useState<ShotDiff | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      setDiff(await api.shotDiff(gameId));
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  }, [gameId]);
  useEffect(() => {
    void reload();
  }, [reload]);

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    try {
      await f();
      await reload();
      onChanged();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const collect = () =>
    run(async () => {
      const [r] = await api.syncShots(gameId);
      store.toast("success", r && r.added > 0 ? t("ui.shots.synced", { n: r.added }) : t("ui.shots.syncedNone"));
    });

  const recollect = () =>
    run(async () => {
      const n = await api.recollectShots(gameId, []);
      store.toast("success", t("ui.diff.recollected", { n }));
    });

  const writeBack = (toFolder: boolean) =>
    run(async () => {
      if (!diff) return;
      let dest: string | null = null;
      if (toFolder) {
        const d = await open({ directory: true, title: t("ui.diff.pickFolder") });
        if (!d || Array.isArray(d)) return;
        dest = d;
      } else if (!(await ask(t("ui.diff.writeBackConfirm", { n: diff.restorable }), { title: t("ui.diff.writeBack"), kind: "warning" }))) {
        return;
      }
      const r = await api.writeBackShots(gameId, [], dest);
      store.toast(
        r.written > 0 ? "success" : "info",
        t("ui.diff.written", { n: r.written, size: bytes(r.bytes) }) + (r.exists + r.noDir > 0 ? t("ui.diff.writtenSkipped", { exists: r.exists, nodir: r.noDir }) : ""),
      );
      if (r.written > 0 && r.dirs[0]) void api.openPath(r.dirs[0]).catch(() => {});
    });

  const trash = diff?.ignored.filter((i) => !i.purged).length ?? 0;
  const purged = (diff?.ignored.length ?? 0) - trash;

  return (
    <Modal
      wide
      title={t("ui.diff.title", { name: gameName })}
      subtitle={t("ui.diff.subtitle")}
      onClose={onClose}
      footer={
        <button className="btn" onClick={onClose}>
          {t("ui.close")}
        </button>
      }
    >
      {!diff ? (
        <p className="muted">{t("ui.loading")}</p>
      ) : (
        <div className="diff-grid">
          <DiffCard tone="ok" icon="check" title={t("ui.diff.synced")} n={diff.synced} body={t("ui.diff.syncedBody")} />
          <DiffCard
            tone={diff.new.length > 0 ? "new" : "ok"}
            icon="camera"
            title={t("ui.diff.new")}
            n={diff.new.length}
            body={diff.duplicates > 0 ? t("ui.diff.newBodyDup", { n: diff.duplicates }) : t("ui.diff.newBody")}
            paths={diff.new.map((f) => f.path)}
            action={
              <button className="btn primary small" disabled={busy || diff.new.length === 0} onClick={() => void collect()}>
                <Icon name="backup" size={14} /> {t("ui.diff.collect", { n: diff.new.length })}
              </button>
            }
          />
          <DiffCard
            tone={diff.ignored.length > 0 ? "warn" : "ok"}
            icon="trash"
            title={t("ui.diff.ignored")}
            n={diff.ignored.length}
            body={t("ui.diff.ignoredBody", { trash, purged })}
            paths={diff.ignored.map((i) => i.path)}
            action={
              <button className="btn small" disabled={busy || diff.ignored.length === 0} onClick={() => void recollect()}>
                <Icon name="undo" size={14} /> {t("ui.diff.recollect", { n: diff.ignored.length })}
              </button>
            }
          />
          <DiffCard
            tone={diff.missing.length > 0 ? "warn" : "ok"}
            icon="export"
            title={t("ui.diff.missing")}
            n={diff.missing.length}
            body={diff.missing.length > 0 ? t("ui.diff.missingBody", { n: diff.restorable }) : t("ui.diff.missingNone")}
            paths={diff.missing.map((s) => s.originalPath)}
            action={
              <>
                <button className="btn small" disabled={busy || diff.restorable === 0} onClick={() => void writeBack(false)} title={t("ui.diff.writeBackHint")}>
                  <Icon name="restore" size={14} /> {t("ui.diff.writeBackN", { n: diff.restorable })}
                </button>
                <button className="btn ghost small" disabled={busy || diff.missing.length === 0} onClick={() => void writeBack(true)}>
                  <Icon name="folder" size={14} /> {t("ui.diff.writeToFolder")}
                </button>
              </>
            }
          />
        </div>
      )}
    </Modal>
  );
}

function DiffCard({
  tone,
  icon,
  title,
  n,
  body,
  paths,
  action,
}: {
  tone: "ok" | "new" | "warn";
  icon: "check" | "camera" | "trash" | "export";
  title: string;
  n: number;
  body: string;
  paths?: string[];
  action?: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <section className={`diff-card ${tone}`} aria-label={title}>
      <header>
        <Icon name={icon} size={16} />
        <h3>{title}</h3>
        <b className="num">{n}</b>
      </header>
      <p>{body}</p>
      {paths && paths.length > 0 && (
        <details>
          <summary>{t("ui.diff.showFiles", { n: paths.length })}</summary>
          <ul className="diff-files">
            {paths.slice(0, 200).map((p) => (
              <li key={p} title={p}>
                {p}
              </li>
            ))}
          </ul>
        </details>
      )}
      {action && <div className="diff-actions">{action}</div>}
    </section>
  );
}
