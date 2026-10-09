import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, imageSrc } from "../api";
import Timeline, { laneColor } from "../components/Timeline";
import { Icon, Menu, StatusBadge, displayName } from "../components/ui";
import { bytes, dateTime, displayPath, hours, relative, shortId } from "../lib/format";
import { layout } from "../lib/timeline";
import { useStore } from "../store";
import type { DiffEntry, HistoryView, LocationRecord, MetadataView, SnapshotNode, TreeEntry } from "../types";
import BackupDialog, { MAX_COMMENT } from "./BackupDialog";
import ExportDialog, { type ExportKind } from "./ExportDialog";
import ImportDialog from "./ImportDialog";
import RestoreDialog from "./RestoreDialog";

type Compare = "disk" | "parent";
type Tab = "changes" | "files";

/**
 * 游戏详情 / 历史（文档 9.1）。
 * 顶部是游戏信息与常用操作（备份、打开存档位置与安装目录、导出、导入）；
 * 下方左侧是分支、标签与快照时间线，右侧是选中快照的详情：恢复、导出此版本、变化与文件。
 */
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
  const [exporting, setExporting] = useState<{ kind: ExportKind; snapshot: string | null } | null>(null);
  const [importing, setImporting] = useState(false);
  const [backingUp, setBackingUp] = useState(false);

  const reload = useCallback(async () => {
    try {
      const h = await api.history(id);
      setHist(h);
      setSelected((cur) => (cur && h.nodes.some((n) => n.id === cur) ? cur : (h.head ?? h.nodes[0]?.id ?? null)));
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
  // 与时间线相同的布局：用来给分支按钮配上对应路线的颜色。
  const graph = useMemo(() => layout(nodes), [nodes]);
  const laneOf = useMemo(() => new Map(graph.rows.map((r) => [r.id, r.col])), [graph]);

  const afterChange = async () => {
    await reload();
    await store.refreshRow(id);
  };
  const fail = (e: unknown) => store.toast("error", errorMessage(e));

  if (error) return <div className="empty"><div className="callout error">{error}</div></div>;
  if (!hist || !row) return <div className="empty">{t("ui.loading")}</div>;
  const name = displayName(row, i18n.language);
  const undoTarget = store.undoable[id];
  const branches = hist.refs.filter((r) => r.kind === "branch");
  const tags = hist.refs.filter((r) => r.kind === "tag");

  return (
    <>
      <div className="toolbar slim">
        <button className="btn ghost" onClick={() => store.go({ name: "library" })} aria-label={t("ui.back")}>
          ‹ {t("ui.nav.library")}
        </button>
      </div>
      <Hero
        hist={hist}
        meta={meta}
        row={row}
        name={name}
        onBackup={() => setBackingUp(true)}
        onExport={(kind, snapshot) => setExporting({ kind, snapshot })}
        onImport={() => setImporting(true)}
        onChanged={afterChange}
        onFail={fail}
      />
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
                fail(e);
              }
            }}
          >
            <Icon name="undo" /> {t("ui.history.undo")}
          </button>
        </div>
      )}
      <div className="detail">
        <section className="trail">
          <div className="trail-head">
            <div className="graph-title">
              <h2>{t("ui.history.graphTitle")}</h2>
              <span className="muted num">{t("ui.history.graphBranches", { n: branches.length })}</span>
              <span className="graph-help" tabIndex={0} title={t("ui.history.graphHint")} aria-label={t("ui.history.graphHint")}>
                ?
              </span>
            </div>
            <div className="ref-chips" role="group" aria-label={t("ui.history.branches")}>
              {branches.map((b) => {
                const col = laneOf.get(b.target);
                return (
                  <button key={b.name} className={`ref-chip branch${b.current ? " current" : ""}`} onClick={() => setSelected(b.target)} title={t("ui.history.goBranch", { name: b.name })}>
                    <i className="lane-dot" style={{ background: graph.columns > 1 && col != null ? laneColor(col) : "var(--faint)" }} />
                    {b.name}
                  </button>
                );
              })}
              {tags.map((g) => (
                <button key={g.name} className="ref-chip" onClick={() => setSelected(g.target)} title={t("ui.history.goTag", { name: g.name })}>
                  <Icon name="tag" size={13} /> {g.name}
                </button>
              ))}
            </div>
            <input className="field" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("ui.history.search")} aria-label={t("ui.history.search")} />
          </div>
          {hist.nodes.length === 0 ? (
            <div className="empty">
              <h2>{t("ui.history.emptyTitle")}</h2>
              <p>{t("ui.history.emptyBody")}</p>
              <div className="actions">
                <button className="btn primary" onClick={() => setBackingUp(true)}>
                  <Icon name="backup" /> {t("ui.history.backupNow")}
                </button>
              </div>
            </div>
          ) : (
            <Timeline nodes={nodes} head={hist.head} selected={selected} onSelect={setSelected} focus={selected} />
          )}
        </section>
        <section className="files">
          {node ? (
            <SnapshotPanel
              key={node.id}
              gameId={id}
              node={node}
              isHead={node.id === hist.head}
              onRestore={(paths) => setRestore({ target: node.id, paths })}
              onExport={() => setExporting({ kind: "files", snapshot: node.id })}
              onChanged={afterChange}
            />
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
      {exporting && <ExportDialog hist={hist} gameName={name} kind={exporting.kind} snapshot={exporting.snapshot} onClose={() => setExporting(null)} />}
      {importing && (
        <ImportDialog
          hist={hist}
          gameName={name}
          onClose={() => setImporting(false)}
          onDone={async (snap) => {
            setImporting(false);
            await afterChange();
            if (snap) setSelected(snap);
          }}
        />
      )}
      {backingUp && (
        <BackupDialog
          rows={[row]}
          onClose={() => {
            setBackingUp(false);
            void afterChange();
          }}
        />
      )}
    </>
  );
}

/** 存档位置的简短名称：最后两级目录。 */
function locationName(l: LocationRecord): string {
  const p = l.kind === "glob" ? l.resolvedPath.replace(/[/\\][^/\\]*[*?[].*$/, "") : l.resolvedPath;
  return p.split(/[/\\]/).filter(Boolean).slice(-2).join("\\");
}

function Hero({
  hist,
  meta,
  row,
  name,
  onBackup,
  onExport,
  onImport,
  onChanged,
  onFail,
}: {
  hist: HistoryView;
  meta: MetadataView | null;
  row: NonNullable<ReturnType<typeof useStore.getState>["rows"][string]>;
  name: string;
  onBackup: () => void;
  onExport: (kind: ExportKind, snapshot: string | null) => void;
  onImport: () => void;
  onChanged: () => void;
  onFail: (e: unknown) => void;
}) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const gameId = hist.game.id;
  const hero = imageSrc(meta?.heroPath);
  const cover = imageSrc(meta?.coverPath ?? meta?.coverThumbPath ?? row.coverThumb);
  const other = name !== hist.game.name ? hist.game.name : null;
  // 未安装的游戏中路径还没确定的位置不能打开。
  const folders = hist.locations.map((l, ord) => ({ l, ord })).filter(({ l }) => l.kind !== "registry" && l.resolvedPath !== "");
  const unconfirmed = hist.locations.filter((l) => !l.confirmed && l.resolvedPath !== "").length;
  const installDir = hist.game.installDir;
  const playtime = meta?.playtimeMin ?? row.playtimeMin;

  const openLocation = (ord: number) => api.openLocation(gameId, ord).catch(onFail);

  return (
    <header className="hero" style={hero ? { ["--hero" as string]: `url("${hero}")` } : undefined}>
      <div className="hero-cover">{cover ? <img src={cover} alt="" /> : <span aria-hidden>{name.slice(0, 1)}</span>}</div>
      <div className="hero-main">
        <div className="hero-title">
          <h1>{name}</h1>
          <StatusBadge row={row} />
          {!row.installed && (
            <span className="tag not-installed" title={t("ui.library.notInstalledHint")}>
              {t("ui.library.notInstalled")}
            </span>
          )}
        </div>
        <p className="hero-meta">
          <span>{t(`ui.platform.${hist.game.platform}`, { defaultValue: hist.game.platform })}</span>
          {other && <span>{other}</span>}
          {meta?.developer && <span>{meta.developer}</span>}
          {playtime != null && playtime > 0 && <span>{t("ui.history.playtime", { h: hours(playtime) })}</span>}
          <span className="num">{t("ui.history.snapshotsCount", { n: hist.nodes.length })}</span>
          {row.lastBackupAt && <span>{t("ui.history.lastBackup", { when: relative(row.lastBackupAt, i18n.language) })}</span>}
          <span className="num">{t("ui.history.saveSize", { size: bytes(row.totalBytes), n: row.files })}</span>
        </p>
        <div className="hero-actions">
          <button className="btn primary" onClick={onBackup}>
            <Icon name="backup" /> {t("ui.history.backupNow")}
          </button>
          {folders.length === 1 ? (
            <button className="btn" onClick={() => void openLocation(folders[0].ord)} title={folders[0].l.resolvedPath}>
              <Icon name="folder" /> {t("ui.history.openSaves")}
            </button>
          ) : folders.length > 1 ? (
            <Menu
              icon="folder"
              label={t("ui.history.openSaves")}
              items={folders.map(({ l, ord }) => ({
                label: (
                  <span className="loc-item">
                    <b>{locationName(l)}</b>
                    <small>
                      {l.tag === "config" ? t("ui.history.config") : t("ui.history.save")}
                      {!l.confirmed && ` · ${t("ui.state.pending")}`}
                    </small>
                  </span>
                ),
                hint: l.resolvedPath,
                onClick: () => void openLocation(ord),
              }))}
            />
          ) : null}
          <button className="btn" onClick={() => installDir && api.openPath(installDir).catch(onFail)} disabled={!installDir} title={installDir ?? t("ui.history.noInstallDir")}>
            <Icon name="game" /> {t("ui.history.openInstall")}
          </button>
          <Menu
            icon="export"
            label={t("ui.transfer.export")}
            items={[
              { label: t("ui.transfer.exportCurrent"), onClick: () => onExport("files", null) },
              { label: t("ui.transfer.exportSnapshot"), onClick: () => onExport("files", hist.head), disabled: hist.nodes.length === 0 },
              { label: t("ui.transfer.exportBundle"), onClick: () => onExport("bundle", null), disabled: hist.nodes.length === 0 },
            ]}
          />
          <button className="btn" onClick={onImport}>
            <Icon name="import" /> {t("ui.transfer.import")}
          </button>
          <Menu
            icon="more"
            className="btn ghost"
            align="right"
            title={t("ui.more")}
            items={[
              { label: t("ui.history.refreshInfo"), onClick: () => void api.refreshInfo(gameId).then(onChanged).catch(onFail) },
              ...(unconfirmed > 0
                ? [
                    {
                      label: t("ui.history.confirm", { n: unconfirmed }),
                      onClick: async () => {
                        try {
                          await api.confirmLocations(gameId);
                          store.toast("success", t("ui.history.confirmed"));
                          onChanged();
                        } catch (e) {
                          onFail(e);
                        }
                      },
                    },
                  ]
                : []),
              {
                label: t("ui.history.remove"),
                danger: true,
                onClick: async () => {
                  if (!(await ask(t("ui.history.removeConfirm", { name }), { kind: "warning" }))) return;
                  try {
                    await api.removeGame(gameId);
                    store.toast("success", t("ui.history.removed", { name }));
                    await store.loadLibrary(false);
                    store.go({ name: "library" });
                  } catch (e) {
                    onFail(e);
                  }
                },
              },
            ]}
          />
        </div>
        {!row.installed && <p className="hero-note">{t("ui.history.notInstalledNote")}</p>}
        {unconfirmed > 0 && <p className="hero-note">{t("ui.history.unconfirmedNote", { n: unconfirmed })}</p>}
      </div>
    </header>
  );
}

function SnapshotPanel({
  gameId,
  node,
  isHead,
  onRestore,
  onExport,
  onChanged,
}: {
  gameId: string;
  node: SnapshotNode;
  isHead: boolean;
  onRestore: (paths: string[]) => void;
  onExport: () => void;
  onChanged: () => void;
}) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const [files, setFiles] = useState<TreeEntry[] | null>(null);
  const [tab, setTab] = useState<Tab>("changes");
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

  const branchNames = node.refs.filter((r) => r.startsWith("heads/")).map((r) => r.slice("heads/".length));
  const tagNames = node.refs.filter((r) => r.startsWith("tags/")).map((r) => r.slice("tags/".length));
  const fail = (e: unknown) => store.toast("error", errorMessage(e));

  // 删除前先预览影响范围（dryRun），在确认框中说明。
  const deleteSnapshot = async () => {
    try {
      const plan = await api.deleteSnapshot(gameId, node.id, true);
      const lines: string[] = [];
      if (plan.rewritten > 0) lines.push(t("ui.history.deleteRewire", { n: plan.rewritten }));
      if (plan.branchesDeleted.length > 0) lines.push(t("ui.history.deleteBranchGone", { names: plan.branchesDeleted.join("、") }));
      if (plan.headBranch) lines.push(t("ui.history.deleteHeadSwitch", { name: plan.headBranch }));
      if (isHead) lines.push(t("ui.history.deleteHereNote"));
      lines.push(t("ui.history.deleteFinal"));
      if (!(await ask(lines.join("\n\n"), { title: t("ui.history.deleteSnapshotConfirm"), kind: "warning" }))) return;
      await api.deleteSnapshot(gameId, node.id, false);
      store.toast("success", t("ui.history.deleted"));
      onChanged();
    } catch (e) {
      fail(e);
    }
  };

  const deleteBranch = async (name: string) => {
    try {
      const plan = await api.deleteBranch(gameId, name, true);
      const lines = [t("ui.history.deleteBranchConfirm", { name, n: plan.removed.length })];
      if (plan.headBranch) lines.push(t("ui.history.deleteHeadSwitch", { name: plan.headBranch }));
      lines.push(t("ui.history.deleteFinal"));
      if (!(await ask(lines.join("\n\n"), { title: t("ui.history.deleteBranch", { name }).replace(/…$/, ""), kind: "warning" }))) return;
      await api.deleteBranch(gameId, name, false);
      store.toast("success", t("ui.history.deleted"));
      onChanged();
    } catch (e) {
      fail(e);
    }
  };

  const deleteTag = async (name: string) => {
    try {
      if (!(await ask(t("ui.history.deleteTagConfirm", { name }), { title: t("ui.history.deleteTag", { name }), kind: "warning" }))) return;
      await api.deleteTag(gameId, name);
      store.toast("success", t("ui.history.deleted"));
      onChanged();
    } catch (e) {
      fail(e);
    }
  };

  const togglePick = (p: string) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(p)) n.delete(p);
      else n.add(p);
      return n;
    });
  const sameAsDisk = isHead && compare === "disk" && diff?.length === 0;

  return (
    <div className="snap">
      <div className="snap-head">
        <h2>{dateTime(node.createdAt, i18n.language)}</h2>
        <div className="snap-tags">
          {isHead && <span className="tag current">{t("ui.history.here")}</span>}
          {node.refs.map((r) => (
            <span key={r} className={`tag${r.startsWith("heads/") ? " branch" : ""}`}>
              {r.replace(/^(heads|tags)\//, "")}
            </span>
          ))}
          {node.trigger !== "manual" && <span className="tag auto">{t(`ui.trigger.${node.trigger}`)}</span>}
        </div>
      </div>
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
        <p className="snap-comment" onDoubleClick={() => setEditing(true)} title={t("ui.history.editHint")}>
          {node.comment || <span className="muted">{t("ui.history.noComment")}</span>}
        </p>
      )}
      <dl className="snap-stats num">
        <div>
          <dt>{t("ui.history.statFiles")}</dt>
          <dd>{node.files}</dd>
        </div>
        <div>
          <dt>{t("ui.history.statSize")}</dt>
          <dd>{bytes(node.bytes)}</dd>
        </div>
        <div>
          <dt>{t("ui.history.statChanged")}</dt>
          <dd>{node.changed}</dd>
        </div>
        {node.playtimeMin != null && (
          <div>
            <dt>{t("ui.history.statPlaytime")}</dt>
            <dd>{hours(node.playtimeMin)} h</dd>
          </div>
        )}
      </dl>
      <p className="snap-id muted num">
        {shortId(node.id, 12)}
        {node.host && ` · ${node.host}`}
      </p>

      <div className="snap-actions">
        <button className="btn primary" onClick={() => onRestore([...picked])} disabled={sameAsDisk && picked.size === 0} title={sameAsDisk ? t("ui.history.sameAsDisk") : undefined}>
          <Icon name="restore" size={16} /> {picked.size > 0 ? t("ui.history.restorePicked", { n: picked.size }) : t("ui.history.restore")}
        </button>
        <button className="btn" onClick={onExport}>
          <Icon name="export" size={16} /> {t("ui.transfer.exportThis")}
        </button>
        <Menu
          icon="more"
          className="btn ghost"
          align="right"
          title={t("ui.more")}
          items={[
            { label: t("ui.history.editComment"), onClick: () => setEditing(true) },
            { label: t("ui.history.addTag"), onClick: () => setRefKind("tag") },
            { label: t("ui.history.addBranch"), onClick: () => setRefKind("branch") },
            ...tagNames.map((name) => ({ label: t("ui.history.deleteTag", { name }), onClick: () => void deleteTag(name) })),
            ...branchNames.map((name) => ({ label: t("ui.history.deleteBranch", { name }), danger: true, onClick: () => void deleteBranch(name) })),
            {
              label: t("ui.history.deleteSnapshot"),
              danger: true,
              disabled: tagNames.length > 0,
              hint: tagNames.length > 0 ? t("ui.history.deleteTagged") : undefined,
              onClick: () => void deleteSnapshot(),
            },
          ]}
        />
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

      <div className="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "changes"} onClick={() => setTab("changes")}>
          {t("ui.history.diff")}
          {diff && diff.length > 0 && <span className="chip-count num">{diff.length}</span>}
        </button>
        <button role="tab" aria-selected={tab === "files"} onClick={() => setTab("files")}>
          {t("ui.history.filesTab")}
          <span className="chip-count num">{files?.length ?? 0}</span>
        </button>
      </div>

      {tab === "changes" ? (
        <div role="tabpanel">
          <div className="chips" role="group" aria-label={t("ui.history.diff")}>
            <button className="chip" aria-pressed={compare === "disk"} onClick={() => setCompare("disk")}>
              {t("ui.history.vsDisk")}
            </button>
            <button className="chip" aria-pressed={compare === "parent"} onClick={() => setCompare("parent")}>
              {t("ui.history.vsParent")}
            </button>
          </div>
          {diff === null && <p className="muted">{t("ui.loading")}</p>}
          {diff?.length === 0 && <p className="muted">{t(compare === "disk" ? "ui.history.noDiffDisk" : "ui.history.noDiff")}</p>}
          <ul className="file-list">
            {diff?.map((d) => (
              <li key={d.path} className={d.status}>
                <span className="mark">{d.status === "added" ? "+" : d.status === "removed" ? "−" : "~"}</span>
                <PathText path={d.path} />
                <span className="muted num">{t(`ui.file.${d.status}`)}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div role="tabpanel">
          <p className="hint">{t("ui.history.pickHint")}</p>
          <ul className="file-list">
            {files?.map((f) => (
              <li key={f.path}>
                <input type="checkbox" className="check" checked={picked.has(f.path)} onChange={() => togglePick(f.path)} aria-label={displayPath(f.path)} />
                <PathText path={f.path} />
                <span className="muted num">{bytes(f.size)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/** 文件路径：目录部分变淡，文件名突出。 */
function PathText({ path }: { path: string }) {
  const p = displayPath(path);
  const i = p.lastIndexOf("/");
  return (
    <span className="path" title={p}>
      {i >= 0 && <span className="dir">{p.slice(0, i + 1)}</span>}
      {p.slice(i + 1)}
    </span>
  );
}
