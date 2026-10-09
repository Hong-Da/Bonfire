import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { dateTime, firstLine, hours, relative, shortId } from "../lib/format";
import { layout } from "../lib/timeline";
import type { SnapshotNode } from "../types";
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
}: {
  nodes: SnapshotNode[];
  head: string | null;
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const g = useMemo(() => layout(nodes), [nodes]);
  const width = PAD * 2 + (g.columns - 1) * COL_W;
  const x = (col: number) => PAD + col * COL_W;
  const y = (row: number) => row * ROW_H + ROW_H / 2;

  const paths = g.edges.map((e) => {
    if (e.fromCol === e.toCol) return `M${x(e.fromCol)} ${y(e.fromRow)} V${y(e.toRow)}`;
    // 沿自己的路线走到父快照上一行，再弯向父快照。
    const bendY = y(e.toRow - 1);
    return `M${x(e.fromCol)} ${y(e.fromRow)} V${bendY} C${x(e.fromCol)} ${bendY + ROW_H * 0.6} ${x(e.toCol)} ${y(e.toRow) - ROW_H * 0.4} ${x(e.toCol)} ${y(e.toRow)}`;
  });

  return (
    <div style={{ position: "relative" }} role="listbox" aria-label={t("ui.history.timeline")}>
      <svg width={width} height={nodes.length * ROW_H} style={{ position: "absolute", left: 0, top: 0, pointerEvents: "none" }} aria-hidden>
        {paths.map((d, i) => (
          <path key={i} d={d} fill="none" stroke="var(--line)" strokeWidth={2.5} strokeLinecap="round" />
        ))}
        {g.rows.map((r) => {
          const n = nodes[r.row];
          return <Flame key={r.id} x={x(r.col)} y={y(r.row)} current={n.id === head} auto={n.trigger !== "manual"} />;
        })}
      </svg>
      {nodes.map((n) => {
        const branches = n.refs.filter((r) => r.startsWith("heads/")).map((r) => r.slice(6));
        const tags = n.refs.filter((r) => r.startsWith("tags/")).map((r) => r.slice(5));
        return (
          <div
            key={n.id}
            className="trail-row"
            role="option"
            aria-selected={selected === n.id}
            tabIndex={0}
            style={{ height: ROW_H }}
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
              </div>
              <span className="comment">{firstLine(n.comment) || t("ui.history.noComment")}</span>
              <span className="stats num">
                <span>{shortId(n.id)}</span>
                <span>{t("ui.history.files", { n: n.files })}</span>
                {n.playtimeMin != null && <span>{t("ui.history.playedAt", { h: hours(n.playtimeMin) })}</span>}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
