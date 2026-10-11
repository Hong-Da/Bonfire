import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { confirm } from "../lib/confirm";
import { Icon, Modal, type IconName } from "../components/ui";
import { bytes, displayPath } from "../lib/format";
import { useStore } from "../store";
import type { ShotDiff } from "../types";
import { ShotThumb } from "./ShotsPanel";

/**
 * 截图与磁盘对比（每次打开时按当前磁盘与相册重新对比）：
 * - 顶部一条比例条与图例，一眼看出两边是否一致；
 * - 需要处理的项排在前面，每项右侧是对应操作：
 *   磁盘上的新截图 → 收集；备份中有、磁盘上没有的 → 写回磁盘（原文件夹还在的写回原位置，
 *   不在本机的——例如从其他电脑导入的——写到本机这个游戏的截图文件夹）或写到指定文件夹；
 *   在回收站中或已彻底删除、磁盘上原图还在的 → 不会再收集，可以重新收集；
 * - 没有差异的项收起为一行。只写入不存在的文件，不覆盖。
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

  const report = (r: { written: number; bytes: number; exists: number; noDir: number; dirs: string[] }) => {
    store.toast(
      r.written > 0 ? "success" : "info",
      t("ui.diff.written", { n: r.written, size: bytes(r.bytes) }) + (r.exists + r.noDir > 0 ? t("ui.diff.writtenSkipped", { exists: r.exists, nodir: r.noDir }) : ""),
    );
    if (r.written > 0 && r.dirs[0]) void api.openPath(r.dirs[0]).catch(() => {});
  };

  // 写回磁盘：原文件夹还在的写回原位置，不在本机的写到本机这个游戏的截图文件夹。
  const writeBack = () =>
    run(async () => {
      if (!diff) return;
      const text = t("ui.diff.writeBackConfirm", { n: writable }) + (diff.home && diff.missing.length > diff.restorable ? "\n\n" + t("ui.diff.writeBackHome", { dir: displayPath(diff.home) }) : "");
      if (!(await confirm(text, { title: t("ui.diff.writeBack"), kind: "warning", ok: t("ui.diff.writeBack") }))) return;
      report(await api.syncShotsToDisk(gameId, diff.missing.map((s) => s.blob)));
    });

  const writeToFolder = () =>
    run(async () => {
      const d = await open({ directory: true, title: t("ui.diff.pickFolder") });
      if (!d || Array.isArray(d)) return;
      report(await api.writeBackShots(gameId, [], d));
    });

  if (!diff)
    return (
      <Modal wide title={t("ui.diff.title", { name: gameName })} onClose={onClose} footer={<button className="btn" onClick={onClose}>{t("ui.close")}</button>}>
        <p className="muted">{t("ui.loading")}</p>
      </Modal>
    );

  const trash = diff.ignored.filter((i) => !i.purged).length;
  const purged = diff.ignored.length - trash;
  const elsewhere = diff.missing.length - diff.restorable;
  const writable = diff.restorable + (diff.home ? elsewhere : 0);
  const parts = [
    { key: "synced", n: diff.synced },
    { key: "new", n: diff.new.length },
    { key: "missing", n: diff.missing.length },
    { key: "ignored", n: diff.ignored.length },
  ];
  const total = parts.reduce((x, p) => x + p.n, 0);
  const clean = diff.new.length === 0 && diff.missing.length === 0 && diff.ignored.length === 0;

  return (
    <Modal
      wide
      title={t("ui.diff.title", { name: gameName })}
      subtitle={t("ui.diff.subtitle")}
      onClose={onClose}
      footer={
        <>
          <button className="btn ghost" onClick={() => void reload()} disabled={busy}>
            <Icon name="scan" size={14} /> {t("ui.diff.refresh")}
          </button>
          <div className="grow" />
          <button className="btn" onClick={onClose}>
            {t("ui.close")}
          </button>
        </>
      }
    >
      <div className="diff-summary">
        <p className={`diff-verdict${clean ? " clean" : ""}`}>
          <Icon name={clean ? "check" : "info"} size={18} />
          {clean ? t("ui.diff.clean", { n: diff.synced }) : t("ui.diff.todo", { n: diff.new.length + diff.missing.length + diff.ignored.length })}
        </p>
        {total > 0 && (
          <div className="diff-bar" role="img" aria-label={parts.map((p) => `${t(`ui.diff.part.${p.key}`)} ${p.n}`).join("，")}>
            {parts
              .filter((p) => p.n > 0)
              .map((p) => (
                <span key={p.key} className={`seg-${p.key}`} style={{ flexGrow: p.n }} />
              ))}
          </div>
        )}
        <ul className="diff-legend">
          {parts.map((p) => (
            <li key={p.key} className={p.n === 0 ? "zero" : undefined}>
              <i className={`dot seg-${p.key}`} /> {t(`ui.diff.part.${p.key}`)} <b className="num">{p.n}</b>
            </li>
          ))}
        </ul>
      </div>

      <div className="diff-rows">
        <DiffRow
          kind="new"
          icon="camera"
          title={t("ui.diff.new")}
          n={diff.new.length}
          body={diff.duplicates > 0 ? t("ui.diff.newBodyDup", { n: diff.duplicates }) : t("ui.diff.newBody")}
          none={t("ui.diff.newNone")}
          files={diff.new.map((f) => f.path)}
          action={
            <button className="btn primary small" disabled={busy} onClick={() => void collect()}>
              <Icon name="backup" size={14} /> {t("ui.diff.collect", { n: diff.new.length })}
            </button>
          }
        />
        <DiffRow
          kind="missing"
          icon="export"
          title={t("ui.diff.missing")}
          n={diff.missing.length}
          body={
            [
              diff.restorable > 0 ? t("ui.diff.missingHere", { n: diff.restorable }) : "",
              elsewhere > 0 ? (diff.home ? t("ui.diff.missingElsewhere", { n: elsewhere, dir: displayPath(diff.home) }) : t("ui.diff.missingNoHome", { n: elsewhere })) : "",
            ]
              .filter(Boolean)
              .join(" ") || t("ui.diff.missingBody0")
          }
          none={t("ui.diff.missingNone")}
          thumbs={diff.missing.slice(0, 8).map((s) => ({ key: s.blob, gameId: s.gameId, blob: s.blob, alt: s.originalPath }))}
          files={diff.missing.map((s) => s.originalPath)}
          action={
            <>
              <button className="btn primary small" disabled={busy || writable === 0} onClick={() => void writeBack()} title={t("ui.diff.writeBackHint")}>
                <Icon name="restore" size={14} /> {t("ui.diff.writeBackN", { n: writable })}
              </button>
              <button className="btn ghost small" disabled={busy} onClick={() => void writeToFolder()}>
                <Icon name="folder" size={14} /> {t("ui.diff.writeToFolder")}
              </button>
            </>
          }
        />
        <DiffRow
          kind="ignored"
          icon="trash"
          title={t("ui.diff.ignored")}
          n={diff.ignored.length}
          body={t("ui.diff.ignoredBody", { trash, purged })}
          none={t("ui.diff.ignoredNone")}
          files={diff.ignored.map((i) => i.path)}
          action={
            <button className="btn small" disabled={busy} onClick={() => void recollect()}>
              <Icon name="undo" size={14} /> {t("ui.diff.recollect", { n: diff.ignored.length })}
            </button>
          }
        />
      </div>
    </Modal>
  );
}

function DiffRow({
  kind,
  icon,
  title,
  n,
  body,
  none,
  files,
  thumbs,
  action,
}: {
  kind: "new" | "missing" | "ignored";
  icon: IconName;
  title: string;
  n: number;
  body: string;
  none: string;
  files: string[];
  thumbs?: { key: string; gameId: string; blob: string; alt: string }[];
  action: ReactNode;
}) {
  const { t } = useTranslation();
  // 没有差异：收起为一行。
  if (n === 0)
    return (
      <section className="diff-row done" aria-label={title}>
        <Icon name="check" size={14} />
        <span>{none}</span>
      </section>
    );
  return (
    <section className={`diff-row ${kind}`} aria-label={title}>
      <span className={`diff-icon seg-${kind}`}>
        <Icon name={icon} size={18} />
      </span>
      <div className="diff-main">
        <h3>
          {title} <b className="num">{n}</b>
        </h3>
        <p>{body}</p>
        {thumbs && thumbs.length > 0 && (
          <ul className="diff-thumbs">
            {thumbs.map((s) => (
              <li key={s.key} title={s.alt}>
                <ShotThumb gameId={s.gameId} blob={s.blob} alt={s.alt} />
              </li>
            ))}
            {n > thumbs.length && <li className="more num">+{n - thumbs.length}</li>}
          </ul>
        )}
        <details>
          <summary>{t("ui.diff.showFiles", { n: files.length })}</summary>
          <ul className="diff-files">
            {files.slice(0, 200).map((p) => (
              <li key={p} title={p}>
                {p}
              </li>
            ))}
          </ul>
        </details>
      </div>
      <div className="diff-actions">{action}</div>
    </section>
  );
}
