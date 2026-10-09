import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { dateTime, firstLine, hours, relative, shortId } from "../lib/format";
import { layout } from "../lib/timeline";
import type { SnapshotNode } from "../types";

/** 选择用的标记：在导出、导入对话框中标出范围内、新增或选中的快照。 */
export type Mark = "in" | "out" | "new" | "pick";
export type TimelineNode = Pick<SnapshotNode, "id" | "parents" | "createdAt" | "trigger" | "comment" | "files"> & Partial<Pick<SnapshotNode, "refs" | "playtimeMin">>;
import { Flame } from "./ui";

export const ROW_H = 68;
const COL_W = 20;
const PAD = 16;

/**
 * 快照时间线：每个快照是路线上的一处篝火，分支是分岔的小路，当前所在的快照发光。
 * 整张图是一个 SVG 垫在列表后面，行高固定。
 */
export default function Timeline({
  nodes,
  head,
  selected,
  onSelect,
  mark,
  compact,
  label,
}: {
  nodes: TimelineNode[];
  head: string | null;
  selected: string | null;
  onSelect: (id: string) => void;
  /** 每个快照的标记；“out”的快照变淡。 */
  mark?: (id: string) => Mark | undefined;
  /** 对话框中使用的紧凑行高。 */
  compact?: boolean;
  label?: string;
}) {
  const { t, i18n } = useTranslation();
  const ROW = compact ? 52 : ROW_H;
  const g = useMemo(() => layout(nodes), [nodes]);
  const width = PAD * 2 + (g.columns - 1) * COL_W;
  const x = (col: number) => PAD + col * COL_W;
  const y = (row: number) => row * ROW + ROW / 2;

  const paths = g.edges.map((e) => {
    if (e.fromCol === e.toCol) return `M${x(e.fromCol)} ${y(e.fromRow)} V${y(e.toRow)}`;
    // 沿自己的路线走到父快照上一行，再弯向父快照。
    const bendY = y(e.toRow - 1);
    return `M${x(e.fromCol)} ${y(e.fromRow)} V${bendY} C${x(e.fromCol)} ${bendY + ROW * 0.6} ${x(e.toCol)} ${y(e.toRow) - ROW * 0.4} ${x(e.toCol)} ${y(e.toRow)}`;
  });

  return (
    <div style={{ position: "relative" }} role="listbox" aria-label={label ?? t("ui.history.timeline")}>
      <svg width={width} height={nodes.length * ROW} style={{ position: "absolute", left: 0, top: 0, pointerEvents: "none" }} aria-hidden>
        {paths.map((d, i) => (
          <path key={i} d={d} fill="none" stroke="var(--line)" strokeWidth={2.5} strokeLinecap="round" />
        ))}
        {g.rows.map((r) => {
          const n = nodes[r.row];
          const m = mark?.(n.id);
          return (
            <g key={r.id} opacity={m === "out" ? 0.3 : 1}>
              {(m === "new" || m === "pick") && <circle cx={x(r.col)} cy={y(r.row)} r={11} fill="none" stroke={m === "pick" ? "var(--ember)" : "var(--moss)"} strokeWidth={2} strokeDasharray={m === "new" ? "3 2" : undefined} />}
              <Flame x={x(r.col)} y={y(r.row)} current={n.id === head} auto={n.trigger !== "manual"} />
            </g>
          );
        })}
      </svg>
      {nodes.map((n) => {
        const refs = n.refs ?? [];
        const m = mark?.(n.id);
        const branches = refs.filter((r) => r.startsWith("heads/")).map((r) => r.slice(6));
        const tags = refs.filter((r) => r.startsWith("tags/")).map((r) => r.slice(5));
        return (
          <div
            key={n.id}
            className={`trail-row${compact ? " compact" : ""}${m ? ` mark-${m}` : ""}`}
            role="option"
            aria-selected={selected === n.id}
            tabIndex={0}
            style={{ height: ROW }}
            onClick={() => onSelect(n.id)}
            onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onSelect(n.id)}
          >
            <div style={{ width }} />
            <div className="entry">
              <div className="line1">
                <span className="when" title={dateTime(n.createdAt, i18n.language)}>
                  {relative(n.createdAt, i18n.language)}
                </span>
                {n.id === head && <span className="tag current">{t("ui.history.here")}</span>}
                {branches.map((b) => (
                  <span key={b} className="tag branch">
                    {b}
                  </span>
                ))}
                {tags.map((tg) => (
                  <span key={tg} className="tag">
                    {tg}
                  </span>
                ))}
                {n.trigger !== "manual" && <span className="tag auto">{t(`ui.trigger.${n.trigger}`)}</span>}
                {m === "new" && <span className="tag new">{t("ui.transfer.newNode")}</span>}
              </div>
              <span className="comment">{firstLine(n.comment) || t("ui.history.noComment")}</span>
              <span className="stats num">
                <span>{shortId(n.id)}</span>
                <span>{t("ui.history.files", { n: n.files })}</span>
                {!compact && n.playtimeMin != null && <span>{t("ui.history.playedAt", { h: hours(n.playtimeMin) })}</span>}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
