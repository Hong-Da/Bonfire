import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, errorMessage, type OnDetached } from "../api";
import { Cover, Modal, displayName } from "../components/ui";
import { bytes, displayPath } from "../lib/format";
import { useStore } from "../store";
import type { BackupItemResult, FileChange, GameRow } from "../types";

export const MAX_COMMENT = 500;

/**
 * 备份确认：逐个游戏写备注（留空自动生成），顶部“统一备注”一键填充，可同时打标签（文档 9.1）。
 * 已是最新的游戏单独列出并自动跳过；只管理截图的游戏只收集截图。
 */
export default function BackupDialog({ rows: picked, onClose }: { rows: GameRow[]; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [comments, setComments] = useState<Record<string, string>>({});
  const [unified, setUnified] = useState("");
  const [withTag, setWithTag] = useState(false);
  const [tag, setTag] = useState("");
  const [detachedMode, setDetachedMode] = useState<"new" | "move">("new");
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState(0);
  const [results, setResults] = useState<BackupItemResult[] | null>(null);
  // 已是最新的游戏跳过（状态以最近一次检查为准，备份时后端还会再判断一次）。
  // 打开时分好，备份完成后行状态刷新也不影响结果列表。
  const [{ skipped, rows }] = useState(() => ({
    skipped: picked.filter((r) => r.state === "up-to-date"),
    rows: picked.filter((r) => r.state !== "up-to-date"),
  }));
  const saveRows = rows.filter((r) => !r.shotsOnly);
  const detached = saveRows.filter((r) => r.detached);
  const tooLong = rows.some((r) => [...(comments[r.id] ?? "")].length > MAX_COMMENT);

  const fillAll = () => setComments(Object.fromEntries(rows.map((r) => [r.id, unified])));

  const start = async () => {
    setRunning(true);
    const unlisten = await api.onBackupProgress((p) => setProgress(p.index + 1));
    try {
      const onDetached: OnDetached = { mode: detachedMode };
      const items = rows.map((r) => ({ gameId: r.id, comment: comments[r.id]?.trim() || undefined, tag: withTag && tag.trim() ? tag.trim() : undefined }));
      const res = await api.runBackup(items, false, onDetached);
      setResults(res);
      await Promise.all(rows.map((r) => store.refreshRow(r.id)));
      const created = res.filter((x) => x.ok && x.result?.result === "created").length;
      store.toast(res.every((x) => x.ok) ? "success" : "error", t("ui.backup.done", { created, failed: res.filter((x) => !x.ok).length }));
      rows.forEach((r) => store.setUndoable(r.id, null));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      unlisten();
      setRunning(false);
    }
  };

  const footer = results ? (
    <button className="btn primary" onClick={onClose}>
      {t("ui.close")}
    </button>
  ) : (
    <>
      <label className="inline" style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <input type="checkbox" className="check" checked={withTag} onChange={(e) => setWithTag(e.target.checked)} />
        {t("ui.backup.withTag")}
      </label>
      {withTag && <input className="field" value={tag} onChange={(e) => setTag(e.target.value)} placeholder={t("ui.backup.tagName")} aria-label={t("ui.backup.tagName")} />}
      <div className="grow" />
      {running && <span className="muted num">{t("ui.backup.progress", { done: progress, total: rows.length })}</span>}
      <button className="btn ghost" onClick={running ? () => api.cancelTask() : onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={start} disabled={running || rows.length === 0 || tooLong || (withTag && !tag.trim())}>
        {t("ui.backup.confirm", { n: rows.length })}
      </button>
    </>
  );

  return (
    <Modal title={t("ui.backup.title")} subtitle={t("ui.backup.subtitle")} footer={footer} onClose={running ? () => {} : onClose} wide>
      {results ? (
        <ul className="note-rows" aria-label={t("ui.backup.results")}>
          {results.map((r) => {
            const row = rows.find((x) => x.id === r.gameId);
            const name = row ? displayName(row, i18n.language) : r.gameId;
            let text: string;
            if (!r.ok) text = t("ui.backup.failed", { msg: r.error?.message ?? "" });
            else if (r.shotsOnly) text = r.shots > 0 ? t("ui.backup.shotsOnlyDone", { n: r.shots }) : t("ui.backup.shotsOnlyNone");
            else if (r.result?.result === "created") text = t("ui.backup.created", { comment: r.result.comment, size: bytes(r.result.newBytes) });
            else text = t("ui.backup.noChanges");
            return (
              <li key={r.gameId} className="note-row">
                <span />
                <div>
                  <div className="head">
                    <strong>{name}</strong>
                  </div>
                  <div className={r.ok ? "" : "callout error"}>{text}</div>
                  {r.result?.result === "created" && r.result.newBranch && <div className="muted">{t("ui.backup.newBranch", { branch: r.result.newBranch })}</div>}
                  {r.result?.result === "created" && r.result.warnings.map((w) => <div key={w} className="callout warn">{w}</div>)}
                  {r.shots > 0 && !r.shotsOnly && <div className="muted">{t("ui.shots.backupCollected", { n: r.shots })}</div>}
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <>
          {rows.length === 0 && <div className="callout same"><p>{t("ui.backup.allUpToDate", { n: skipped.length })}</p></div>}
          {saveRows.length > 1 && (
          <div className="row">
            <label htmlFor="unified">{t("ui.backup.unified")}</label>
            <div className="inline">
              <input id="unified" className="field" style={{ flex: 1 }} value={unified} onChange={(e) => setUnified(e.target.value)} placeholder={t("ui.backup.commentPlaceholder")} />
              <button className="btn" onClick={fillAll} disabled={!unified.trim()}>
                {t("ui.backup.fillAll")}
              </button>
            </div>
          </div>
          )}
          {detached.length > 0 && (
            <div className="callout warn">
              <p style={{ margin: "0 0 6px" }}>{t("ui.backup.detachedIntro", { names: detached.map((r) => displayName(r, i18n.language)).join("、") })}</p>
              <label style={{ display: "block" }}>
                <input type="radio" name="detached" checked={detachedMode === "new"} onChange={() => setDetachedMode("new")} /> {t("ui.backup.detachedNew")}
              </label>
              <label style={{ display: "block" }}>
                <input type="radio" name="detached" checked={detachedMode === "move"} onChange={() => setDetachedMode("move")} /> {t("ui.backup.detachedMove")}
              </label>
            </div>
          )}
          <ul className="note-rows">
            {rows.map((r) => (
              <NoteRow key={r.id} row={r} value={comments[r.id] ?? ""} onChange={(v) => setComments((c) => ({ ...c, [r.id]: v }))} />
            ))}
          </ul>
          {skipped.length > 0 && (
            <section className="skip-list" aria-label={t("ui.backup.skippedTitle", { n: skipped.length })}>
              <h3>{t("ui.backup.skippedTitle", { n: skipped.length })}</h3>
              <p className="hint">{t("ui.backup.skippedHint")}</p>
              <ul>
                {skipped.map((r) => (
                  <li key={r.id}>
                    <Cover row={r} size="note" />
                    <span>{displayName(r, i18n.language)}</span>
                    <span className="tag">{t("ui.backup.upToDate")}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </>
      )}
    </Modal>
  );
}

function NoteRow({ row, value, onChange }: { row: GameRow; value: string; onChange: (v: string) => void }) {
  const { t, i18n } = useTranslation();
  const [files, setFiles] = useState<FileChange[] | null>(null);
  const [open, setOpen] = useState(false);
  const len = [...value].length;
  const name = displayName(row, i18n.language);
  const changed = row.summary.added + row.summary.modified + row.summary.deleted;

  useEffect(() => {
    if (open && files === null) api.gameChanges(row.id).then(setFiles).catch(() => setFiles([]));
  }, [open, files, row.id]);

  if (row.shotsOnly)
    return (
      <li className="note-row">
        <Cover row={row} size="note" />
        <div>
          <div className="head">
            <strong>{name}</strong>
            <span className="muted">{t("ui.backup.shotsOnlyRow")}</span>
          </div>
        </div>
      </li>
    );
  return (
    <li className="note-row">
      <Cover row={row} size="note" />
      <div>
        <div className="head">
          <strong>{name}</strong>
          <span className="muted">
            {row.state === "never-backed-up"
              ? t("ui.backup.firstBackup", { n: row.files })
              : row.state === "up-to-date"
                ? t("ui.backup.upToDate")
                : t("ui.library.changedFiles", { n: changed, size: bytes(row.summary.changedBytes) })}
          </span>
        </div>
        <textarea
          className="field comment-edit"
          rows={2}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t("ui.backup.commentPlaceholder")}
          aria-label={t("ui.backup.commentFor", { name })}
        />
        <div className={`counter num${len > MAX_COMMENT ? " over" : ""}`}>
          {len}/{MAX_COMMENT}
        </div>
        {changed > 0 && (
          <details onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}>
            <summary>{t("ui.backup.showFiles")}</summary>
            <ul>
              {files === null && <li className="muted">{t("ui.loading")}</li>}
              {files?.map((f) => (
                <li key={f.path}>
                  {t(`ui.file.${f.status}`)} {displayPath(f.path)}
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </li>
  );
}
