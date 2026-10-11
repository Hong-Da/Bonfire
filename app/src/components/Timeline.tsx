import { useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { dateTime, firstLine, hours, relative, shortId } from "../lib/format";
import { layout } from "../lib/timeline";
import type { SnapshotNode } from "../types";

/** 选择用的标记：在导出、导入对话框中标出范围内、新增、选中或与导入内容相同的快照。 */
export type Mark = "in" | "out" | "new" | "pick" | "same";
export type TimelineNode = Pick<SnapshotNode, "id" | "parents" | "createdAt" | "trigger" | "comment" | "files"> & Partial<Pick<SnapshotNode, "refs" | "playtimeMin">>;
import { Flame, Icon } from "./ui";

export const ROW_H = 68;
const COL_W = 20;
const PAD = 16;

/** 每条路线（分支）的颜色，按列循环；与 styles.css 中的 --lane-* 一致。 */
export const LANES = ["var(--lane-0)", "var(--lane-1)", "var(--lane-2)", "var(--lane-3)", "var(--lane-4)", "var(--lane-5)"];
export const laneColor = (col: number) => LANES[col % LANES.length];

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
  focus,
  focusKey,
  shots,
  wide,
  onOpen,
  multi,
}: {
  nodes: TimelineNode[];
  head: string | null;
  selected: string | null;
  /** 点击快照；`mods` 为按住的 Ctrl / Shift（多选）。 */
  onSelect: (id: string, mods?: { ctrl: boolean; shift: boolean }) => void;
  /** 每个快照的标记；“out”的快照变淡。 */
  mark?: (id: string) => Mark | undefined;
  /** 对话框中使用的紧凑行高。 */
  compact?: boolean;
  label?: string;
  /** 变化时把这个快照滚动到可见位置。 */
  focus?: string | null;
  /** 改变它可以再次滚动到 `focus`。 */
  focusKey?: number;
  /** 每个快照之前拍的截图数（详情页）。 */
  shots?: Map<string, number>;
  /** 大图查看：路线间距更宽。 */
  wide?: boolean;
  /** 双击快照（大图查看中“查看这个快照”）。 */
  onOpen?: (id: string) => void;
  /** 多选的快照（详情页 Ctrl / Shift 点选，用于一次删除多个）。 */
  multi?: Set<string>;
}) {
  const { t, i18n } = useTranslation();
  const ROW = compact ? 52 : ROW_H;
  const g = useMemo(() => layout(nodes), [nodes]);
  const colW = wide ? 34 : COL_W;
  const width = PAD * 2 + (g.columns - 1) * colW;
  const x = (col: number) => PAD + col * colW;
  const y = (row: number) => row * ROW + ROW / 2;
  const colOf = useMemo(() => new Map(g.rows.map((r) => [r.id, r.col])), [g]);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!focus) return;
    const el = box.current?.querySelector<HTMLElement>(`[data-id="${focus}"]`);
    el?.scrollIntoView?.({ block: "nearest", behavior: "smooth" });
  }, [focus, focusKey]);

  const paths = g.edges.map((e) => {
    if (e.fromCol === e.toCol) return `M${x(e.fromCol)} ${y(e.fromRow)} V${y(e.toRow)}`;
    // 沿自己的路线走到父快照上一行，再弯向父快照。
    const bendY = y(e.toRow - 1);
    return `M${x(e.fromCol)} ${y(e.fromRow)} V${bendY} C${x(e.fromCol)} ${bendY + ROW * 0.6} ${x(e.toCol)} ${y(e.toRow) - ROW * 0.4} ${x(e.toCol)} ${y(e.toRow)}`;
  });

  return (
    <div style={{ position: "relative" }} role="listbox" aria-label={label ?? t("ui.history.timeline")} aria-multiselectable={multi ? true : undefined} ref={box}>
      <svg width={width} height={nodes.length * ROW} style={{ position: "absolute", left: 0, top: 0, pointerEvents: "none" }} aria-hidden>
        {paths.map((d, i) => (
          // 每条路线一种颜色（只有一条路线时也用第一种颜色），便于分辨分支；不用灰色，避免像“已停用”。
          <path key={i} d={d} fill="none" stroke={laneColor(g.edges[i].fromCol)} strokeOpacity={0.75} strokeWidth={wide ? 3 : 2.5} strokeLinecap="round" />
        ))}
        {g.rows.map((r) => {
          const n = nodes[r.row];
          const m = mark?.(n.id);
          return (
            <g key={r.id} opacity={m === "out" ? 0.3 : 1}>
              {(m === "new" || m === "pick" || m === "same") && (
                <circle cx={x(r.col)} cy={y(r.row)} r={12} fill="none" stroke={m === "new" ? "var(--moss)" : "var(--ember)"} strokeWidth={m === "same" ? 3 : 2} strokeDasharray={m === "new" ? "3 2" : undefined} />
              )}
              <Flame x={x(r.col)} y={y(r.row)} current={n.id === head} auto={n.trigger !== "manual"} color={laneColor(r.col)} />
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
            data-id={n.id}
            className={`trail-row${compact ? " compact" : ""}${m ? ` mark-${m}` : ""}${multi?.has(n.id) ? " multi" : ""}`}
            role="option"
            aria-selected={multi && multi.size > 0 ? multi.has(n.id) : selected === n.id}
            tabIndex={0}
            style={{ height: ROW }}
            onClick={(e) => onSelect(n.id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })}
            onDoubleClick={onOpen ? () => onOpen(n.id) : undefined}
            onKeyDown={(e) => {
              if (e.key === " ") onSelect(n.id);
              else if (e.key === "Enter") (onOpen ?? onSelect)(n.id);
            }}
          >
            <div style={{ width }} />
            <div className="entry">
              <div className="line1">
                {multi?.has(n.id) && (
                  <span className="multi-check" aria-hidden>
                    <Icon name="check" size={12} />
                  </span>
                )}
                <span className="when" title={dateTime(n.createdAt, i18n.language)}>
                  {relative(n.createdAt, i18n.language)}
                </span>
                {n.id === head && <span className="tag current">{t("ui.history.here")}</span>}
                {branches.map((b) => (
                  <span key={b} className="tag branch" style={{ borderColor: laneColor(colOf.get(n.id) ?? 0), color: laneColor(colOf.get(n.id) ?? 0) }}>
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
                {m === "same" && <span className="tag same">{t("ui.transfer.sameNode")}</span>}
                {m === "pick" && <span className="tag pick">{t("ui.transfer.pickNode")}</span>}
              </div>
              <span className="comment">{firstLine(n.comment) || t("ui.history.noComment")}</span>
              <span className="stats num">
                <span>{shortId(n.id)}</span>
                <span>{t("ui.history.files", { n: n.files })}</span>
                {!compact && n.playtimeMin != null && <span>{t("ui.history.playedAt", { h: hours(n.playtimeMin) })}</span>}
                {!!shots?.get(n.id) && (
                  <span className="shot-badge" title={t("ui.shots.badge", { n: shots.get(n.id) })}>
                    <Icon name="camera" size={12} /> {shots.get(n.id)}
                  </span>
                )}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
