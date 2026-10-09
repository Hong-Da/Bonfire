import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import { Cover, Icon, Modal, displayName } from "../components/ui";
import { bytes, dateTime, shortId } from "../lib/format";
import { useStore } from "../store";
import type { ArchiveImportItem, ArchivePreview, BundlePreview, FilesSource, GameRow } from "../types";
import { MAX_COMMENT } from "./BackupDialog";

/**
 * 在游戏库中一次导出多个游戏：选中游戏的全部快照与分支打包为一个合集包（.bonfire）。
 * 还没有快照的游戏不能选。
 */
export function CollectionExportDialog({ rows, initial, onClose }: { rows: GameRow[]; initial: string[]; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const exportable = rows.filter((r) => r.snapshots > 0);
  const [picked, setPicked] = useState<Set<string>>(() => new Set(initial.filter((id) => exportable.some((r) => r.id === id))));
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const chosen = exportable.filter((r) => picked.has(r.id));
  const snapshots = chosen.reduce((n, r) => n + r.snapshots, 0);

  const toggle = (id: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });

  const run = async () => {
    const stamp = new Date().toISOString().slice(0, 10).replaceAll("-", "");
    const dest = await save({ defaultPath: `${t("ui.collection.fileName")}-${stamp}.bonfire`, filters: [{ name: t("ui.transfer.bundle"), extensions: ["bonfire"] }] });
    if (!dest) return;
    setBusy(true);
    try {
      const r = await api.exportCollection(
        chosen.map((x) => x.id),
        dest,
      );
      setDone(r.dest);
      store.toast("success", t("ui.collection.exported", { n: r.games.length, size: bytes(r.bytes) }));
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const footer = done ? (
    <>
      <button className="btn" onClick={() => api.openPath(done)}>
        <Icon name="folder" /> {t("ui.transfer.showFile")}
      </button>
      <button className="btn primary" onClick={onClose}>
        {t("ui.done")}
      </button>
    </>
  ) : (
    <>
      <span className="count num">{t("ui.collection.summary", { n: chosen.length, s: snapshots })}</span>
      <div className="grow" />
      <button className="btn ghost" onClick={onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={() => void run()} disabled={busy || chosen.length === 0}>
        <Icon name="export" /> {t("ui.transfer.exportTo")}
      </button>
    </>
  );

  return (
    <Modal title={t("ui.collection.exportTitle")} subtitle={t("ui.collection.exportIntro")} onClose={onClose} footer={footer}>
      <div className="pick-head">
        <label className="radio">
          <input
            type="checkbox"
            checked={chosen.length === exportable.length && exportable.length > 0}
            ref={(el) => {
              if (el) el.indeterminate = chosen.length > 0 && chosen.length < exportable.length;
            }}
            onChange={() => setPicked(chosen.length === exportable.length ? new Set() : new Set(exportable.map((r) => r.id)))}
          />
          {t("ui.library.selectAll")}
        </label>
      </div>
      <ul className="pick-list">
        {rows.map((r) => {
          const disabled = r.snapshots === 0;
          return (
            <li key={r.id} className={disabled ? "disabled" : picked.has(r.id) ? "on" : undefined}>
              <label>
                <input type="checkbox" checked={picked.has(r.id)} disabled={disabled} onChange={() => toggle(r.id)} />
                <Cover row={r} />
                <span className="pick-name">
                  <b>{displayName(r, i18n.language)}</b>
                  <small>{t(`ui.platform.${r.platform}`, { defaultValue: r.platform })}</small>
                </span>
                <span className="muted num">{disabled ? t("ui.collection.noSnapshots") : t("ui.library.snapshots", { n: r.snapshots })}</span>
              </label>
            </li>
          );
        })}
      </ul>
    </Modal>
  );
}

type Loaded = { kind: "archive"; path: string; preview: ArchivePreview } | { kind: "files"; path: string; source: FilesSource };

/** 合集包中的游戏能否导入、导入后有没有变化。 */
function importable(p: BundlePreview): boolean {
  if (p.missing.length > 0) return false;
  return p.createGame || p.newSnapshots > 0 || p.refs.some((r) => r.action === "create" || r.action === "fast-forward" || r.action === "diverged");
}

/**
 * 在游戏库中导入：
 * - 历史包或合集包（.bonfire）：列出包中的游戏，勾选要导入的；
 * - 别人分享的存档文件（篝火导出的 zip 或文件夹）：按附带的说明找到游戏。
 * 游戏库中没有的游戏自动加入，标为“未安装”。导入只写入仓库，不改动存档目录。
 */
export function CollectionImportDialog({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [results, setResults] = useState<ArchiveImportItem[] | null>(null);
  const [filesDone, setFilesDone] = useState<string | null>(null);
  const [comment, setComment] = useState("");

  const choose = async (what: "archive" | "zip" | "folder") => {
    setError(null);
    setResults(null);
    setFilesDone(null);
    const path =
      what === "folder"
        ? await open({ directory: true, title: t("ui.transfer.pickFolder") })
        : await open({ filters: what === "archive" ? [{ name: t("ui.transfer.bundle"), extensions: ["bonfire"] }] : [{ name: "Zip", extensions: ["zip"] }] });
    if (!path || Array.isArray(path)) return;
    setBusy(true);
    try {
      if (what === "archive") {
        const preview = await api.inspectArchive(path);
        setLoaded({ kind: "archive", path, preview });
        setPicked(new Set(preview.games.filter((g) => importable(g.preview)).map((g) => g.key)));
      } else {
        setLoaded({ kind: "files", path, source: await api.inspectFilesSource(path) });
      }
    } catch (e) {
      setLoaded(null);
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (key: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(key)) n.delete(key);
      else n.add(key);
      return n;
    });

  const run = async () => {
    if (!loaded) return;
    setBusy(true);
    try {
      if (loaded.kind === "archive") {
        const items = await api.importArchive(
          loaded.path,
          [...picked].map((key) => ({ key, target: null })),
        );
        setResults(items);
        const failed = items.filter((i) => i.error).length;
        if (failed > 0) store.toast("error", t("ui.collection.importFailed", { n: failed }));
        else store.toast("success", t("ui.collection.importDone", { ok: items.length }));
      } else {
        const r = await api.importFilesAuto(loaded.path, comment.trim() || null);
        setFilesDone(r.gameId);
        store.toast("success", t("ui.collection.filesDone", { name: r.gameName }));
      }
      onDone();
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const finished = results !== null || filesDone !== null;
  const canImport =
    !busy &&
    !!loaded &&
    !finished &&
    (loaded.kind === "archive" ? picked.size > 0 : !loaded.source.preview?.identical && (loaded.source.preview?.unmapped.length ?? 0) === 0 && [...comment].length <= MAX_COMMENT);

  const footer = finished ? (
    <>
      {filesDone && (
        <button
          className="btn"
          onClick={() => {
            onClose();
            store.go({ name: "game", id: filesDone });
          }}
        >
          {t("ui.collection.openGame")}
        </button>
      )}
      <button className="btn primary" onClick={onClose}>
        {t("ui.done")}
      </button>
    </>
  ) : (
    <>
      <button className="btn ghost" onClick={onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={() => void run()} disabled={!canImport}>
        <Icon name="import" /> {loaded?.kind === "archive" ? t("ui.collection.importPicked", { n: picked.size }) : t("ui.transfer.import")}
      </button>
    </>
  );

  const resultOf = (key: string) => results?.find((r) => r.key === key);

  return (
    <Modal title={t("ui.collection.importTitle")} subtitle={t("ui.collection.importIntro")} onClose={onClose} footer={footer}>
      <div className="collection-import">
        <div className={`source-picks${loaded ? " compact" : ""}`}>
          {(
            [
              ["archive", "branch", "ui.collection.chooseArchive", "ui.collection.chooseArchiveHint"],
              ["zip", "import", "ui.transfer.chooseZip", "ui.collection.chooseZipHint"],
              ["folder", "folder", "ui.transfer.chooseFolder", "ui.collection.chooseFolderHint"],
            ] as const
          ).map(([what, icon, key, hint]) => (
            <button key={what} className="source-pick" onClick={() => void choose(what)} disabled={busy}>
              <Icon name={icon} />
              <span>
                <b>{t(key)}</b>
                {!loaded && <small>{t(hint)}</small>}
              </span>
            </button>
          ))}
        </div>
        {error && <div className="callout error">{error}</div>}
        {loaded && <p className="source-path" title={loaded.path}>{loaded.path}</p>}

        {loaded?.kind === "archive" && (
          <>
            <p className="hint">
              {t("ui.transfer.bundleFrom", { host: loaded.preview.host, time: dateTime(loaded.preview.createdAt, i18n.language) })}
            </p>
            <div className="pick-head">
              <span className="muted">{t("ui.collection.games", { n: loaded.preview.games.length })}</span>
            </div>
            <ul className="pick-list">
              {loaded.preview.games.map(({ key, preview: p }) => {
                const ok = importable(p);
                const res = resultOf(key);
                return (
                  <li key={key} className={!ok ? "disabled" : picked.has(key) ? "on" : undefined}>
                    <label>
                      <input type="checkbox" checked={picked.has(key)} disabled={!ok || finished} onChange={() => toggle(key)} />
                      <span className="pick-name">
                        <b>{p.gameName}</b>
                        <small>
                          {p.createGame ? <span className="tag found-new">{t("ui.collection.stateNew")}</span> : <span className="tag found-managed">{t("ui.collection.stateLocal")}</span>}
                        </small>
                      </span>
                      <span className="pick-state num">
                        {res ? (
                          res.error ? (
                            <span className="danger-text" title={res.error}>
                              {res.error}
                            </span>
                          ) : (
                            <span className="ok-text">
                              <Icon name="check" size={14} /> {t("ui.collection.resultOk", { n: res.report?.importedSnapshots ?? 0 })}
                              {res.report?.createdGame && ` · ${t("ui.collection.resultAdded")}`}
                            </span>
                          )
                        ) : p.missing.length > 0 ? (
                          <span className="muted">{t("ui.collection.missing")}</span>
                        ) : p.newSnapshots > 0 ? (
                          t("ui.collection.newSnapshots", { n: p.newSnapshots })
                        ) : (
                          <span className="muted">{t("ui.collection.upToDate")}</span>
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          </>
        )}

        {loaded?.kind === "files" && (
          <div className="files-target">
            <p>
              <b>{t("ui.collection.filesFor", { name: loaded.source.manifest.gameName })}</b>
            </p>
            <p className="hint">
              {t("ui.transfer.bundleFrom", { host: loaded.source.manifest.host, time: dateTime(loaded.source.manifest.exportedAt, i18n.language) })}
              {loaded.source.manifest.snapshot && ` · ${shortId(loaded.source.manifest.snapshot)}`}
            </p>
            {loaded.source.preview?.identical ? (
              <div className="callout same">
                <p>{t("ui.collection.identical", { name: loaded.source.manifest.gameName, id: shortId(loaded.source.preview.identical) })}</p>
              </div>
            ) : (
              <p className="hint">{t(loaded.source.gameId ? "ui.collection.filesExisting" : "ui.collection.filesNewGame")}</p>
            )}
            {!loaded.source.preview?.identical && !filesDone && (
              <input className="field" value={comment} onChange={(e) => setComment(e.target.value)} placeholder={t("ui.transfer.commentPlaceholder")} aria-label={t("ui.history.comment")} />
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
