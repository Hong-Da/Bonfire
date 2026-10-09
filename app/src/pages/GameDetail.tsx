import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, imageSrc } from "../api";
import Timeline from "../components/Timeline";
import { Icon, StatusBadge, displayName } from "../components/ui";
import { bytes, dateTime, displayPath, hours, shortId } from "../lib/format";
import { useStore } from "../store";
import type { DiffEntry, HistoryView, MetadataView, SnapshotNode, TreeEntry } from "../types";
import RestoreDialog from "./RestoreDialog";
import { MAX_COMMENT } from "./BackupDialog";

type Compare = "disk" | "parent";

/** 游戏详情 / 历史：左侧分支与标签，中间快照时间线，右侧选中快照的文件与差异（文档 9.1）。 */
export default function GameDetail({ id }: { id: string }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const row = store.rows[id];
  const [hist, setHist] = useState<HistoryView | null>(null);
  const [meta, setMeta] = useState<MetadataView | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [restore, setRestore] = useState<{ target: string; paths: string[] } | null>(null);

  const reload = useCallback(async () => {
    try {
      const h = await api.history(id);
      setHist(h);
      setSelected((cur) => cur ?? h.head ?? h.nodes[0]?.id ?? null);
      setMeta(await api.gameInfo(id));
      const undo = await api.pendingUndo(id);
      store.setUndoable(id, undo?.target ?? null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [id]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const nodes = useMemo(() => {
    if (!hist) return [];
    const q = query.trim().toLowerCase();
    return q ? hist.nodes.filter((n) => n.comment.toLowerCase().includes(q) || n.refs.some((r) => r.toLowerCase().includes(q))) : hist.nodes;
  }, [hist, query]);
  const node = hist?.nodes.find((n) => n.id === selected) ?? null;

  const afterChange = async () => {
    await reload();
    await store.refreshRow(id);
  };

  if (error) return <div className="empty"><div className="callout error">{error}</div></div>;
  if (!hist || !row) return <div className="empty">{t("ui.loading")}</div>;
  const name = displayName(row, i18n.language);
  const undoTarget = store.undoable[id];

  return (
    <>
      <div className="toolbar">
        <button className="btn ghost" onClick={() => store.go({ name: "library" })} aria-label={t("ui.back")}>
          ‹ {t("ui.nav.library")}
        </button>
        <h1>{name}</h1>
        <StatusBadge row={row} />
        <div className="grow" />
        <button className="btn" onClick={() => api.refreshInfo(id).then(reload).catch((e) => store.toast("error", errorMessage(e)))}>
          {t("ui.history.refreshInfo")}
        </button>
      </div>
      {undoTarget && (
        <div className="undo-bar" role="status">
          <span>{t("ui.history.restoredFrom", { id: shortId(undoTarget) })}</span>
          <div className="grow" />
          <button
            className="btn"
            onClick={async () => {
              try {
                await api.undoRestore(id);
                store.setUndoable(id, null);
                store.toast("success", t("ui.history.undone"));
                await afterChange();
              } catch (e) {
                store.toast("error", errorMessage(e));
              }
            }}
          >
            <Icon name="undo" /> {t("ui.history.undo")}
          </button>
        </div>
      )}
      <div className="detail">
        <RefsPanel hist={hist} meta={meta} name={name} selected={selected} onSelect={setSelected} onChanged={afterChange} />
        <section className="trail">
          <div className="trail-search">
            <input className="field" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("ui.history.search")} aria-label={t("ui.history.search")} />
          </div>
          {hist.nodes.length === 0 ? (
            <div className="empty">
              <h2>{t("ui.history.emptyTitle")}</h2>
              <p>{t("ui.history.emptyBody")}</p>
            </div>
          ) : (
            <Timeline nodes={nodes} head={hist.head} selected={selected} onSelect={setSelected} />
          )}
        </section>
        <section className="files">
          {node ? (
            <SnapshotPanel key={node.id} gameId={id} node={node} isHead={node.id === hist.head} onRestore={(paths) => setRestore({ target: node.id, paths })} onChanged={afterChange} />
          ) : (
            <p className="muted">{t("ui.history.pick")}</p>
          )}
        </section>
      </div>
      {restore && (
        <RestoreDialog
          row={row}
          target={restore.target}
          paths={restore.paths}
          onClose={() => setRestore(null)}
          onDone={async () => {
            setRestore(null);
            await afterChange();
          }}
        />
      )}
    </>
  );
}

function RefsPanel({ hist, meta, name, selected, onSelect, onChanged }: { hist: HistoryView; meta: MetadataView | null; name: string; selected: string | null; onSelect: (id: string) => void; onChanged: () => void }) {
  const { t } = useTranslation();
  const store = useStore();
  const branches = hist.refs.filter((r) => r.kind === "branch");
  const tags = hist.refs.filter((r) => r.kind === "tag");
  const unconfirmed = hist.locations.filter((l) => !l.confirmed);
  const cover = imageSrc(meta?.coverPath);

  return (
    <section className="refs">
      {cover && <img src={cover} alt={name} style={{ width: "100%", borderRadius: 6, marginBottom: 12 }} />}
      {meta && (
        <p className="muted" style={{ fontSize: 12, margin: "0 0 8px" }}>
          {meta.developer && <span style={{ display: "block" }}>{meta.developer}</span>}
          {meta.playtimeMin != null && <span style={{ display: "block" }}>{t("ui.history.playtime", { h: hours(meta.playtimeMin) })}</span>}
        </p>
      )}
      <h3>{t("ui.history.branches")}</h3>
      <ul>
        {branches.map((b) => (
          <li key={b.name}>
            <button onClick={() => onSelect(b.target)} aria-current={b.current} title={shortId(b.target)}>
              <Icon name="branch" size={14} /> {b.name}
            </button>
          </li>
        ))}
      </ul>
      {tags.length > 0 && (
        <>
          <h3>{t("ui.history.tags")}</h3>
          <ul>
            {tags.map((g) => (
              <li key={g.name}>
                <button onClick={() => onSelect(g.target)} aria-current={selected === g.target}>
                  <Icon name="tag" size={14} /> {g.name}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      <h3>{t("ui.history.locations")}</h3>
      <ul className="file-list">
        {hist.locations.map((l, i) => (
          <li key={i} title={l.resolvedPath}>
            <Icon name="folder" size={14} />
            <button className="btn ghost small path" style={{ justifyContent: "flex-start" }} onClick={() => api.openPath(l.kind === "glob" ? l.resolvedPath.replace(/[/\\][^/\\]*[*?[].*$/, "") : l.resolvedPath)}>
              {l.resolvedPath.split(/[/\\]/).slice(-2).join("/")}
            </button>
            <span className="muted">{l.confirmed ? (l.tag === "config" ? t("ui.history.config") : "") : t("ui.state.pending")}</span>
          </li>
        ))}
      </ul>
      {unconfirmed.length > 0 && (
        <button
          className="btn small"
          style={{ marginTop: 8 }}
          onClick={async () => {
            try {
              await api.confirmLocations(hist.game.id);
              store.toast("success", t("ui.history.confirmed"));
              onChanged();
            } catch (e) {
              store.toast("error", errorMessage(e));
            }
          }}
        >
          <Icon name="check" size={14} /> {t("ui.history.confirm", { n: unconfirmed.length })}
        </button>
      )}
      <button
        className="btn ghost small danger"
        style={{ marginTop: 24 }}
        onClick={async () => {
          if (!(await ask(t("ui.history.removeConfirm", { name }), { kind: "warning" }))) return;
          try {
            await api.removeGame(hist.game.id);
            store.toast("success", t("ui.history.removed", { name }));
            await store.loadLibrary(false);
            store.go({ name: "library" });
          } catch (e) {
            store.toast("error", errorMessage(e));
          }
        }}
      >
        <Icon name="trash" size={14} /> {t("ui.history.remove")}
      </button>
    </section>
  );
}

function SnapshotPanel({ gameId, node, isHead, onRestore, onChanged }: { gameId: string; node: SnapshotNode; isHead: boolean; onRestore: (paths: string[]) => void; onChanged: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [files, setFiles] = useState<TreeEntry[] | null>(null);
  const [compare, setCompare] = useState<Compare>("disk");
  const [diff, setDiff] = useState<DiffEntry[] | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [editing, setEditing] = useState(false);
  const [comment, setComment] = useState(node.comment);
  const [refName, setRefName] = useState("");
  const [refKind, setRefKind] = useState<"tag" | "branch" | null>(null);

  useEffect(() => {
    api.snapshotFiles(gameId, node.id).then(setFiles).catch(() => setFiles([]));
  }, [gameId, node.id]);

  useEffect(() => {
    setDiff(null);
    const parent = node.parents[0];
    if (compare === "parent" && !parent) {
      setDiff([]);
      return;
    }
    const p = compare === "disk" ? api.diff(gameId, node.id, null) : api.diff(gameId, parent!, node.id);
    p.then(setDiff).catch(() => setDiff([]));
  }, [gameId, node.id, compare]);

  const saveComment = async () => {
    try {
      await api.setComment(gameId, node.id, comment);
      setEditing(false);
      onChanged();
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  };

  const createRef = async () => {
    if (!refKind || !refName.trim()) return;
    try {
      await api.createRef(gameId, refKind, refName.trim(), node.id);
      setRefKind(null);
      setRefName("");
      onChanged();
    } catch (e) {
      store.toast("error", errorMessage(e));
    }
  };

  const togglePick = (p: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(p)) n.delete(p);
      else n.add(p);
      return n;
    });

  return (
    <>
      <h3>{dateTime(node.createdAt, i18n.language)}</h3>
      {editing ? (
        <>
          <textarea className="field comment-edit" rows={3} value={comment} onChange={(e) => setComment(e.target.value)} aria-label={t("ui.history.comment")} autoFocus />
          <div className={`counter num${[...comment].length > MAX_COMMENT ? " over" : ""}`}>
            {[...comment].length}/{MAX_COMMENT}
          </div>
          <div className="actions">
            <button className="btn small primary" onClick={saveComment} disabled={[...comment].length > MAX_COMMENT}>
              {t("ui.save")}
            </button>
            <button className="btn small ghost" onClick={() => setEditing(false)}>
              {t("ui.cancel")}
            </button>
          </div>
        </>
      ) : (
        <p onDoubleClick={() => setEditing(true)} title={t("ui.history.editHint")} style={{ whiteSpace: "pre-wrap", margin: "4px 0" }}>
          {node.comment || <span className="muted">{t("ui.history.noComment")}</span>}
        </p>
      )}
      <p className="muted num" style={{ fontSize: 12 }}>
        {t("ui.history.stats", { files: node.files, size: bytes(node.bytes), changed: node.changed })}
        {node.host && <span style={{ marginLeft: 8 }}>{node.host}</span>}
      </p>
      <div className="actions">
        <button className="btn primary small" onClick={() => onRestore([...picked])} disabled={isHead && compare === "disk" && diff?.length === 0 && picked.size === 0}>
          <Icon name="restore" size={16} /> {picked.size > 0 ? t("ui.history.restorePicked", { n: picked.size }) : t("ui.history.restore")}
        </button>
        {!editing && (
          <button className="btn small" onClick={() => setEditing(true)}>
            <Icon name="edit" size={14} /> {t("ui.history.editComment")}
          </button>
        )}
        <button className="btn small" onClick={() => setRefKind("tag")}>
          <Icon name="tag" size={14} /> {t("ui.history.addTag")}
        </button>
        <button className="btn small" onClick={() => setRefKind("branch")}>
          <Icon name="branch" size={14} /> {t("ui.history.addBranch")}
        </button>
      </div>
      {refKind && (
        <form
          className="actions"
          onSubmit={(e) => {
            e.preventDefault();
            void createRef();
          }}
        >
          <input className="field" autoFocus value={refName} onChange={(e) => setRefName(e.target.value)} placeholder={t(refKind === "tag" ? "ui.history.tagName" : "ui.history.branchName")} aria-label={t(refKind === "tag" ? "ui.history.tagName" : "ui.history.branchName")} />
          <button className="btn small primary" type="submit" disabled={!refName.trim()}>
            {t("ui.create")}
          </button>
          <button className="btn small ghost" type="button" onClick={() => setRefKind(null)}>
            {t("ui.cancel")}
          </button>
        </form>
      )}

      <h3 style={{ marginTop: 18 }}>{t("ui.history.diff")}</h3>
      <div className="chips" role="group" aria-label={t("ui.history.diff")}>
        <button className="chip" aria-pressed={compare === "disk"} onClick={() => setCompare("disk")}>
          {t("ui.history.vsDisk")}
        </button>
        <button className="chip" aria-pressed={compare === "parent"} onClick={() => setCompare("parent")}>
          {t("ui.history.vsParent")}
        </button>
      </div>
      {diff === null && <p className="muted">{t("ui.loading")}</p>}
      {diff?.length === 0 && <p className="muted">{t("ui.history.noDiff")}</p>}
      <ul className="file-list" style={{ marginTop: 6 }}>
        {diff?.map((d) => (
          <li key={d.path} className={d.status}>
            <span className="mark">{d.status === "added" ? "+" : d.status === "removed" ? "−" : "~"}</span>
            <span className="path">{displayPath(d.path)}</span>
            <span className="muted num">{t(`ui.file.${d.status}`)}</span>
          </li>
        ))}
      </ul>

      <h3 style={{ marginTop: 18 }}>{t("ui.history.filesTitle", { n: files?.length ?? 0 })}</h3>
      <p className="hint">{t("ui.history.pickHint")}</p>
      <ul className="file-list">
        {files?.map((f) => (
          <li key={f.path}>
            <input type="checkbox" className="check" checked={picked.has(f.path)} onChange={() => togglePick(f.path)} aria-label={displayPath(f.path)} />
            <span className="path" title={f.path}>
              {displayPath(f.path)}
            </span>
            <span className="muted num">{bytes(f.size)}</span>
          </li>
        ))}
      </ul>
    </>
  );
}
