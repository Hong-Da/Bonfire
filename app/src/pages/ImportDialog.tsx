import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import Timeline, { type Mark, type TimelineNode } from "../components/Timeline";
import { Icon, Modal } from "../components/ui";
import { bytes, dateTime, firstLine, shortId } from "../lib/format";
import { useStore } from "../store";
import type { BundlePreview, FilesPreview, HistoryView, ImportPlace, RefPlan } from "../types";
import { MAX_COMMENT } from "./BackupDialog";

type Loaded = { kind: "bundle"; path: string; preview: BundlePreview } | { kind: "files"; path: string; preview: FilesPreview };
type Placement = "independent" | "fork" | "continue";

/**
 * 导入：
 * - 历史包：按快照 ID 判断每个分支与本地的关系（新建 / 相同 / 后续更新 / 已包含 / 分叉），分支图中虚线圈出新增的快照；
 * - 存档文件（zip 或文件夹）：生成一个新快照，默认放在新分支上作为独立起点，也可从选中的快照分出或接在某个分支之后。
 * 导入只写入仓库，不改动存档目录；要使用导入的存档，在历史中选中后恢复。
 */
export default function ImportDialog({ hist, gameName, onClose, onDone }: { hist: HistoryView; gameName: string; onClose: () => void; onDone: (snapshot: string | null) => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const gameId = hist.game.id;
  const branches = hist.refs.filter((r) => r.kind === "branch");
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [placement, setPlacement] = useState<Placement>("independent");
  const [parent, setParent] = useState<string | null>(hist.head);
  const [onto, setOnto] = useState(hist.branch);
  const [branchName, setBranchName] = useState("");
  const [comment, setComment] = useState("");

  const choose = async (what: "bundle" | "zip" | "folder") => {
    setError(null);
    const path =
      what === "folder"
        ? await open({ directory: true, title: t("ui.transfer.pickFolder") })
        : await open({ filters: what === "bundle" ? [{ name: t("ui.transfer.bundle"), extensions: ["bonfire"] }] : [{ name: "Zip", extensions: ["zip"] }] });
    if (!path || Array.isArray(path)) return;
    setBusy(true);
    try {
      if (what === "bundle") setLoaded({ kind: "bundle", path, preview: await api.inspectBundle(path, gameId) });
      else setLoaded({ kind: "files", path, preview: await api.inspectFiles(gameId, path) });
    } catch (e) {
      setLoaded(null);
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  // 分支图：本地历史，加上历史包中本地没有的快照。
  const graph = useMemo(() => {
    const local: TimelineNode[] = hist.nodes;
    if (loaded?.kind !== "bundle") return local;
    const labels = new Map<string, string[]>();
    for (const r of loaded.preview.refs) {
      if (r.action === "same" || r.action === "contained" || r.action === "tag-conflict") continue;
      const name = r.kind === "tag" ? `tags/${r.name}` : `heads/${r.newName ?? r.name}`;
      labels.set(r.target, [...(labels.get(r.target) ?? []), name]);
    }
    const extra: TimelineNode[] = loaded.preview.nodes.filter((n) => !n.known).map((n) => ({ ...n, refs: labels.get(n.id) ?? [] }));
    return [...extra, ...local].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  }, [hist.nodes, loaded]);

  const newIds = useMemo(() => new Set(loaded?.kind === "bundle" ? loaded.preview.nodes.filter((n) => !n.known).map((n) => n.id) : []), [loaded]);
  const mark = (id: string): Mark | undefined => {
    if (newIds.has(id)) return "new";
    if (loaded?.kind === "files") {
      if (loaded.preview.identical === id) return "pick";
      if (placement === "fork" && id === parent) return "pick";
      if (placement === "continue" && id === branches.find((b) => b.name === onto)?.target) return "pick";
    }
    return undefined;
  };

  const place = (): ImportPlace =>
    placement === "continue" ? { kind: "continue", branch: onto } : { kind: "newBranch", name: branchName.trim() || null, parent: placement === "fork" ? parent : null };

  const canImport =
    !busy &&
    !!loaded &&
    (loaded.kind === "bundle"
      ? loaded.preview.missing.length === 0 && loaded.preview.newSnapshots + loaded.preview.refs.filter((r) => r.action === "create" || r.action === "fast-forward").length > 0
      : loaded.preview.unmapped.length === 0 && !loaded.preview.identical && [...comment].length <= MAX_COMMENT && (placement !== "fork" || !!parent));

  const run = async () => {
    if (!loaded) return;
    setBusy(true);
    try {
      if (loaded.kind === "bundle") {
        const r = await api.importBundle(loaded.path, gameId);
        store.toast("success", t("ui.transfer.importedBundle", { n: r.importedSnapshots }));
        onDone(null);
      } else {
        const r = await api.importFiles(gameId, loaded.path, place(), comment.trim() || null);
        store.toast("success", t("ui.transfer.importedFiles", { branch: r.branch ?? "" }));
        onDone(r.snapshot);
      }
    } catch (e) {
      store.toast("error", errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const footer = (
    <>
      <button className="btn ghost" onClick={onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={() => void run()} disabled={!canImport}>
        <Icon name="import" /> {t("ui.transfer.import")}
      </button>
    </>
  );

  return (
    <Modal title={t("ui.transfer.importTitle", { name: gameName })} subtitle={t("ui.transfer.importSafe")} onClose={onClose} footer={footer} wide>
      <div className="transfer">
        <div className="transfer-options">
          <div className="source-picks">
            <button className="btn" onClick={() => void choose("bundle")} disabled={busy}>
              <Icon name="branch" /> {t("ui.transfer.chooseBundle")}
            </button>
            <button className="btn" onClick={() => void choose("zip")} disabled={busy}>
              <Icon name="import" /> {t("ui.transfer.chooseZip")}
            </button>
            <button className="btn" onClick={() => void choose("folder")} disabled={busy}>
              <Icon name="folder" /> {t("ui.transfer.chooseFolder")}
            </button>
          </div>
          {!loaded && !error && <p className="hint">{t("ui.transfer.importIntro")}</p>}
          {error && <div className="callout error">{error}</div>}
          {loaded && <p className="source-path" title={loaded.path}>{loaded.path}</p>}
          {loaded?.kind === "bundle" && <BundleSummary p={loaded.preview} />}
          {loaded?.kind === "files" && (
            <>
              <FilesSummary p={loaded.preview} />
              {!loaded.preview.identical && loaded.preview.unmapped.length === 0 && (
                <fieldset>
                  <legend>{t("ui.transfer.placeTitle")}</legend>
                  <label className="radio">
                    <input type="radio" checked={placement === "independent"} onChange={() => setPlacement("independent")} /> {t("ui.transfer.placeIndependent")}
                  </label>
                  <label className="radio">
                    <input type="radio" checked={placement === "fork"} onChange={() => setPlacement("fork")} disabled={hist.nodes.length === 0} /> {t("ui.transfer.placeFork")}
                  </label>
                  {placement === "fork" && (
                    <p className="hint">
                      {t("ui.transfer.placeForkHint")} {parent && <b className="num">{shortId(parent)}</b>}
                    </p>
                  )}
                  <label className="radio">
                    <input type="radio" checked={placement === "continue"} onChange={() => setPlacement("continue")} disabled={branches.length === 0} /> {t("ui.transfer.placeContinue")}
                  </label>
                  {placement === "continue" && (
                    <select className="field" value={onto} onChange={(e) => setOnto(e.target.value)} aria-label={t("ui.transfer.placeContinue")}>
                      {branches.map((b) => (
                        <option key={b.name} value={b.name}>
                          {b.name}
                        </option>
                      ))}
                    </select>
                  )}
                  {placement !== "continue" && (
                    <input className="field" value={branchName} onChange={(e) => setBranchName(e.target.value)} placeholder={t("ui.transfer.branchPlaceholder")} aria-label={t("ui.transfer.branchName")} />
                  )}
                  <input className="field" value={comment} onChange={(e) => setComment(e.target.value)} placeholder={t("ui.transfer.commentPlaceholder")} aria-label={t("ui.history.comment")} />
                </fieldset>
              )}
            </>
          )}
        </div>
        <div className="transfer-graph">
          {graph.length === 0 ? (
            <p className="muted">{t("ui.transfer.noHistory")}</p>
          ) : (
            <Timeline
              nodes={graph}
              head={hist.head}
              selected={null}
              onSelect={(id) => {
                if (loaded?.kind === "files" && !newIds.has(id)) {
                  setPlacement("fork");
                  setParent(id);
                }
              }}
              mark={mark}
              compact
              label={t("ui.transfer.graph")}
            />
          )}
          {loaded?.kind === "files" && graph.length > 0 && <p className="hint graph-hint">{t("ui.transfer.graphForkHint")}</p>}
        </div>
      </div>
    </Modal>
  );

  function BundleSummary({ p }: { p: BundlePreview }) {
    return (
      <div className="import-summary">
        <p>
          {t("ui.transfer.bundleFrom", { host: p.host, time: dateTime(p.createdAt, i18n.language) })}
          <br />
          <b className="num">{t("ui.transfer.bundleCounts", { total: p.nodes.length, n: p.newSnapshots })}</b>
        </p>
        {p.missing.length > 0 && <div className="callout error">{t("ui.transfer.missing", { n: p.missing.length })}</div>}
        {p.locationMismatch && <div className="callout warn">{t("ui.transfer.locationMismatch")}</div>}
        <ul className="ref-plans">
          {p.refs.map((r) => (
            <RefLine key={`${r.kind}/${r.name}`} r={r} />
          ))}
        </ul>
      </div>
    );
  }

  function RefLine({ r }: { r: RefPlan }) {
    const text =
      r.action === "diverged"
        ? t("ui.transfer.action.diverged", { name: r.newName ?? "", fork: r.forkPoint ? shortId(r.forkPoint) : "—" })
        : t(`ui.transfer.action.${r.action}`);
    return (
      <li className={`ref-plan ${r.action}`}>
        <Icon name={r.kind === "tag" ? "tag" : "branch"} size={14} />
        <b>{r.name}</b>
        <span>{text}</span>
      </li>
    );
  }

  function FilesSummary({ p }: { p: FilesPreview }) {
    return (
      <div className="import-summary">
        {p.manifest && (
          <p>
            {t("ui.transfer.filesFrom", { game: p.manifest.gameName, host: p.manifest.host, time: dateTime(p.manifest.exportedAt, i18n.language) })}
            {p.manifest.comment && (
              <>
                <br />
                <span className="muted">{firstLine(p.manifest.comment)}</span>
              </>
            )}
          </p>
        )}
        <p className="num">{t("ui.transfer.filesCount", { n: p.files, size: bytes(p.bytes) })}</p>
        <ul className="folder-map">
          {p.folders.map((f) => (
            <li key={f.folder} className={f.ord == null ? "bad" : undefined}>
              <span className="path">{f.folder || "."}</span>
              <span>→</span>
              <span>{f.ord == null ? t("ui.transfer.unmapped") : locationLabel(f.ord)}</span>
              <span className="muted num">{t("ui.history.files", { n: f.files })}</span>
            </li>
          ))}
        </ul>
        {p.unmapped.length > 0 && <div className="callout error">{t("ui.transfer.unmappedHint", { n: p.unmapped.length })}</div>}
        {p.identical && <div className="callout">{t("ui.transfer.identical", { id: shortId(p.identical) })}</div>}
      </div>
    );
  }

  function locationLabel(ord: number): string {
    const l = hist.locations[ord];
    return l ? l.resolvedPath.split(/[/\\]/).filter(Boolean).slice(-2).join("/") : `#${ord + 1}`;
  }
}
