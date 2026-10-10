import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { bytes, dateTime, hours, shortId } from "../lib/format";
import { layout } from "../lib/timeline";
import type { HistoryView } from "../types";
import Timeline, { laneColor } from "./Timeline";
import { Icon } from "./ui";

/**
 * 分支图大图：全屏显示完整的分支历史，路线更宽，可搜索、按分支跳转；
 * 右侧是选中快照的摘要，双击快照或点“查看这个快照”回到详情页并选中它。Esc 关闭。
 */
export default function GraphViewer({
  hist,
  title,
  initial,
  shots,
  onPick,
  onClose,
}: {
  hist: HistoryView;
  title: string;
  initial: string | null;
  shots?: Map<string, number>;
  onPick: (id: string) => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [cur, setCur] = useState<string | null>(initial);
  const [query, setQuery] = useState("");
  const [focusKey, setFocusKey] = useState(0);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const nodes = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? hist.nodes.filter((n) => n.comment.toLowerCase().includes(q) || n.refs.some((r) => r.toLowerCase().includes(q))) : hist.nodes;
  }, [hist, query]);
  const graph = useMemo(() => layout(nodes), [nodes]);
  const laneOf = useMemo(() => new Map(graph.rows.map((r) => [r.id, r.col])), [graph]);
  const node = hist.nodes.find((n) => n.id === cur) ?? null;
  const branches = hist.refs.filter((r) => r.kind === "branch");
  const tags = hist.refs.filter((r) => r.kind === "tag");
  const jump = (id: string) => {
    setCur(id);
    setFocusKey((k) => k + 1);
  };

  return (
    <div className="graph-viewer" role="dialog" aria-modal="true" aria-label={t("ui.history.bigGraphTitle", { name: title })}>
      <header>
        <h2>{t("ui.history.bigGraphTitle", { name: title })}</h2>
        <span className="muted num">
          {t("ui.history.graphBranches", { n: branches.length })} · {t("ui.history.snapshotsCount", { n: hist.nodes.length })}
        </span>
        <div className="ref-chips" role="group" aria-label={t("ui.history.branches")}>
          {branches.map((b) => (
            <button key={b.name} className={`ref-chip branch${b.current ? " current" : ""}`} onClick={() => jump(b.target)} title={t("ui.history.goBranch", { name: b.name })}>
              <i className="lane-dot" style={{ background: laneColor(laneOf.get(b.target) ?? 0) }} />
              {b.name}
            </button>
          ))}
          {tags.map((g) => (
            <button key={g.name} className="ref-chip" onClick={() => jump(g.target)} title={t("ui.history.goTag", { name: g.name })}>
              <Icon name="tag" size={13} /> {g.name}
            </button>
          ))}
        </div>
        <div className="grow" />
        <input className="field" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("ui.history.search")} aria-label={t("ui.history.search")} />
        <button className="btn" onClick={onClose} aria-label={t("ui.close")}>
          <Icon name="close" /> {t("ui.close")}
        </button>
      </header>
      <div className="graph-body">
        <div className="graph-scroll">
          {nodes.length === 0 ? (
            <p className="muted">{t("ui.history.noMatch")}</p>
          ) : (
            <Timeline nodes={nodes} head={hist.head} selected={cur} onSelect={setCur} onOpen={onPick} focus={cur} focusKey={focusKey} shots={shots} wide />
          )}
        </div>
        <aside className="graph-side">
          {node ? (
            <>
              <h3 className="num">{dateTime(node.createdAt, i18n.language)}</h3>
              <div className="snap-tags">
                {node.id === hist.head && <span className="tag current">{t("ui.history.here")}</span>}
                {node.refs.map((r) => (
                  <span key={r} className={`tag${r.startsWith("heads/") ? " branch" : ""}`} style={r.startsWith("heads/") ? { color: laneColor(laneOf.get(node.id) ?? 0), borderColor: laneColor(laneOf.get(node.id) ?? 0) } : undefined}>
                    {r.replace(/^(heads|tags)\//, "")}
                  </span>
                ))}
                {node.trigger !== "manual" && <span className="tag auto">{t(`ui.trigger.${node.trigger}`)}</span>}
              </div>
              <p className="snap-comment">{node.comment || <span className="muted">{t("ui.history.noComment")}</span>}</p>
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
              <button className="btn primary" onClick={() => onPick(node.id)}>
                {t("ui.history.openSnapshot")}
              </button>
              <p className="hint">{t("ui.history.bigGraphHint")}</p>
            </>
          ) : (
            <p className="muted">{t("ui.history.pick")}</p>
          )}
        </aside>
      </div>
    </div>
  );
}
