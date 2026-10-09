import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "../api";
import Timeline, { type Mark } from "../components/Timeline";
import { Icon, Modal } from "../components/ui";
import { bytes, dateTime, firstLine, shortId } from "../lib/format";
import { ancestors, rangeOf } from "../lib/timeline";
import { useStore } from "../store";
import type { BundleScope, ExportFormat, HistoryView } from "../types";

export type ExportKind = "files" | "bundle";
type Source = "current" | "snapshot";
type Scope = "all" | "branches" | "range";

/**
 * 导出：
 * - 存档文件：当前存档或分支图上选中的快照，zip 或文件夹，拿到的人可以直接使用；
 * - 历史包（.bonfire）：全部分支、勾选的分支，或在分支图上点选的一段历史，可在另一台电脑或另一个人的篝火中导入。
 */
export default function ExportDialog({ hist, gameName, kind: initialKind, snapshot, onClose }: { hist: HistoryView; gameName: string; kind: ExportKind; snapshot: string | null; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const store = useStore();
  const gameId = hist.game.id;
  const nodes = hist.nodes;
  const branches = hist.refs.filter((r) => r.kind === "branch");
  const [kind, setKind] = useState<ExportKind>(nodes.length === 0 ? "files" : initialKind);
  const [source, setSource] = useState<Source>(snapshot ? "snapshot" : "current");
  const [picked, setPicked] = useState<string | null>(snapshot ?? hist.head ?? nodes[0]?.id ?? null);
  const [format, setFormat] = useState<ExportFormat>("zip");
  const [scope, setScope] = useState<Scope>("all");
  const [names, setNames] = useState<Set<string>>(new Set(branches.filter((b) => b.current).map((b) => b.name)));
  const [range, setRange] = useState<{ from: string | null; to: string | null }>({ from: null, to: snapshot ?? hist.head });
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);

  const included = useMemo(() => {
    if (kind !== "bundle") return null;
    if (scope === "all") return ancestors(nodes, hist.refs.map((r) => r.target));
    if (scope === "branches") return ancestors(nodes, branches.filter((b) => names.has(b.name)).map((b) => b.target));
    return range.to ? rangeOf(nodes, range.from, range.to) : new Set<string>();
  }, [kind, scope, names, range, nodes, hist.refs]);

  const mark = (id: string): Mark | undefined => {
    if (kind === "files") return source === "snapshot" && id === picked ? "pick" : source === "current" ? "out" : undefined;
    if (scope === "range" && (id === range.from || id === range.to)) return "pick";
    return included?.has(id) ? "in" : "out";
  };

  const onNode = (id: string) => {
    if (kind === "files") {
      setSource("snapshot");
      setPicked(id);
      return;
    }
    if (scope !== "range") return;
    // 先点终点；再点终点之前的快照作为起点；点别处则重新选终点。
    setRange((r) => {
      if (!r.to || id === r.to) return { from: null, to: id };
      if (ancestors(nodes, [r.to]).has(id)) return { from: id, to: r.to };
      return { from: null, to: id };
    });
  };

  const pickedNode = nodes.find((n) => n.id === picked);
  const count = included?.size ?? 0;
  const canExport = !busy && (kind === "files" ? source === "current" || !!picked : count > 0);

  const run = async () => {
    setBusy(true);
    try {
      if (kind === "files") {
        const spec = source === "snapshot" ? picked : null;
        const name = await api.exportDefaultName(gameId, spec);
        const dest =
          format === "zip"
            ? await save({ defaultPath: `${name}.zip`, filters: [{ name: "Zip", extensions: ["zip"] }] })
            : await open({ directory: true, title: t("ui.transfer.pickParent") });
        if (!dest || Array.isArray(dest)) return;
        const r = await api.exportFiles(gameId, spec, dest, format);
        setDone(r.dest);
        store.toast("success", t("ui.transfer.exportedFiles", { n: r.files, size: bytes(r.bytes) }));
      } else {
        const s: BundleScope =
          scope === "all" ? { kind: "all" } : scope === "branches" ? { kind: "branches", names: [...names] } : { kind: "range", from: range.from, to: range.to! };
        const name = await api.exportDefaultName(gameId, null);
        const dest = await save({ defaultPath: `${name}.bonfire`, filters: [{ name: t("ui.transfer.bundle"), extensions: ["bonfire"] }] });
        if (!dest) return;
        const r = await api.exportBundle(gameId, s, dest);
        setDone(r.dest);
        store.toast("success", t("ui.transfer.exportedBundle", { n: r.snapshots, size: bytes(r.bytes) }));
      }
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
      <button className="btn ghost" onClick={onClose}>
        {t("ui.cancel")}
      </button>
      <button className="btn primary" onClick={() => void run()} disabled={!canExport}>
        <Icon name="export" /> {t("ui.transfer.exportTo")}
      </button>
    </>
  );

  return (
    <Modal title={t("ui.transfer.exportTitle", { name: gameName })} onClose={onClose} footer={footer} wide>
      <div className="transfer">
        <div className="transfer-options">
          <div className="segmented" role="radiogroup" aria-label={t("ui.transfer.kind")}>
            <button role="radio" aria-checked={kind === "files"} onClick={() => setKind("files")}>
              {t("ui.transfer.files")}
            </button>
            <button role="radio" aria-checked={kind === "bundle"} onClick={() => setKind("bundle")} disabled={nodes.length === 0}>
              {t("ui.transfer.bundle")}
            </button>
          </div>
          <p className="hint">{t(kind === "files" ? "ui.transfer.filesHint" : "ui.transfer.bundleHint")}</p>

          {kind === "files" ? (
            <>
              <fieldset>
                <legend>{t("ui.transfer.what")}</legend>
                <label className="radio">
                  <input type="radio" checked={source === "current"} onChange={() => setSource("current")} /> {t("ui.transfer.current")}
                </label>
                <label className="radio">
                  <input type="radio" checked={source === "snapshot"} onChange={() => setSource("snapshot")} disabled={nodes.length === 0} /> {t("ui.transfer.snapshot")}
                </label>
                {source === "snapshot" && pickedNode && (
                  <div className="picked">
                    <b>{dateTime(pickedNode.createdAt, i18n.language)}</b>
                    <span>{firstLine(pickedNode.comment) || t("ui.history.noComment")}</span>
                    <span className="muted num">{shortId(pickedNode.id)} · {t("ui.history.files", { n: pickedNode.files })}</span>
                  </div>
                )}
                {source === "snapshot" && <p className="hint">{t("ui.transfer.pickOnGraph")}</p>}
              </fieldset>
              <fieldset>
                <legend>{t("ui.transfer.format")}</legend>
                <label className="radio">
                  <input type="radio" checked={format === "zip"} onChange={() => setFormat("zip")} /> {t("ui.transfer.zip")}
                </label>
                <label className="radio">
                  <input type="radio" checked={format === "folder"} onChange={() => setFormat("folder")} /> {t("ui.transfer.folder")}
                </label>
              </fieldset>
            </>
          ) : (
            <fieldset>
              <legend>{t("ui.transfer.scope")}</legend>
              <label className="radio">
                <input type="radio" checked={scope === "all"} onChange={() => setScope("all")} /> {t("ui.transfer.scopeAll", { n: branches.length })}
              </label>
              <label className="radio">
                <input type="radio" checked={scope === "branches"} onChange={() => setScope("branches")} /> {t("ui.transfer.scopeBranches")}
              </label>
              {scope === "branches" && (
                <div className="branch-picks">
                  {branches.map((b) => (
                    <label key={b.name} className="radio">
                      <input
                        type="checkbox"
                        checked={names.has(b.name)}
                        onChange={() =>
                          setNames((s) => {
                            const n = new Set(s);
                            if (n.has(b.name)) n.delete(b.name);
                            else n.add(b.name);
                            return n;
                          })
                        }
                      />
                      <Icon name="branch" size={14} /> {b.name}
                      {b.current && <span className="tag current">{t("ui.history.here")}</span>}
                    </label>
                  ))}
                </div>
              )}
              <label className="radio">
                <input type="radio" checked={scope === "range"} onChange={() => setScope("range")} /> {t("ui.transfer.scopeRange")}
              </label>
              {scope === "range" && (
                <div className="range-picks">
                  <p className="hint">{t("ui.transfer.rangeHint")}</p>
                  <div>
                    <span className="muted">{t("ui.transfer.rangeTo")}</span> {range.to ? <b className="num">{shortId(range.to)}</b> : "—"}
                  </div>
                  <div>
                    <span className="muted">{t("ui.transfer.rangeFrom")}</span> {range.from ? <b className="num">{shortId(range.from)}</b> : t("ui.transfer.rangeFromStart")}
                    {range.from && (
                      <button className="btn ghost small" onClick={() => setRange((r) => ({ ...r, from: null }))}>
                        {t("ui.transfer.rangeClear")}
                      </button>
                    )}
                  </div>
                </div>
              )}
              <p className="summary num">{t("ui.transfer.includes", { n: count })}</p>
            </fieldset>
          )}
        </div>
        <div className="transfer-graph">
          {nodes.length === 0 ? (
            <p className="muted">{t("ui.transfer.noHistory")}</p>
          ) : (
            <Timeline nodes={nodes} head={hist.head} selected={kind === "files" && source === "snapshot" ? picked : null} onSelect={onNode} mark={mark} compact label={t("ui.transfer.graph")} />
          )}
        </div>
      </div>
    </Modal>
  );
}
