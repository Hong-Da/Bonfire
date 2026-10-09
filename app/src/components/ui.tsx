import { useEffect, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { imageSrc } from "../api";
import { useStore } from "../store";
import type { GameRow } from "../types";

type IconName =
  | "library" | "settings" | "scan" | "backup" | "restore" | "branch" | "tag" | "search" | "folder"
  | "close" | "plus" | "undo" | "cloud" | "edit" | "trash" | "check";

const PATHS: Record<IconName, string> = {
  library: "M4 5h4v14H4zM10 5h4v14h-4zM16.5 5.5l3.8 1-3.6 13.5-3.8-1z",
  settings: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zm8 3-2-.6-.5-1.3 1-1.8-1.8-1.8-1.8 1-1.3-.5L12 4h-2l-.6 2-1.3.5-1.8-1-1.8 1.8 1 1.8-.5 1.3L2 12v2l2 .6.5 1.3-1 1.8 1.8 1.8 1.8-1 1.3.5.6 2h2l.6-2 1.3-.5 1.8 1 1.8-1.8-1-1.8.5-1.3 2-.6z",
  scan: "M20 12a8 8 0 1 1-2.3-5.7M20 4v4h-4",
  backup: "M12 4v10m0 0-4-4m4 4 4-4M5 16v3h14v-3",
  restore: "M4 12a8 8 0 1 0 2.3-5.7M4 4v4h4M12 8v4l3 2",
  branch: "M7 4v16M7 8c0 4 10 3 10 8M17 16v4M7 4a1.5 1.5 0 1 0 0 .1M17 20a1.5 1.5 0 1 0 0 .1",
  tag: "M4 4h7l9 9-7 7-9-9zM8 8h.01",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zm9 16-4-4",
  folder: "M3 6h6l2 2h10v11H3z",
  close: "M6 6l12 12M18 6 6 18",
  plus: "M12 5v14M5 12h14",
  undo: "M9 7 4 12l5 5M4 12h10a6 6 0 0 1 0 12",
  cloud: "M7 18a4 4 0 0 1-.5-8A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z",
  edit: "M4 20h4L19 9l-4-4L4 16zM14 6l4 4",
  trash: "M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13",
  check: "M5 12l5 5 9-10",
};

export function Icon({ name, size = 18, label }: { name: IconName; size?: number; label?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <path d={PATHS[name]} />
    </svg>
  );
}

/** 时间线上的篝火：快照节点。当前所在的快照发光。 */
export function Flame({ x, y, current, auto }: { x: number; y: number; current?: boolean; auto?: boolean }) {
  const fill = auto ? "var(--faint)" : "var(--ember)";
  return (
    <g transform={`translate(${x - 7} ${y - 9})`}>
      {current && <circle cx={7} cy={10} r={10} fill="var(--glow)" opacity={0.35} />}
      <path d="M7 0C9 4 13 6 13 11a6 6 0 0 1-12 0c0-3 2-4 3-6 .5 2 1.5 3 3 3C6.5 6 6 3 7 0z" fill={fill} />
      <path d="M7 9c1.5 1.6 3 2.6 3 4.4a3 3 0 0 1-6 0C4 11.8 5.5 10.6 7 9z" fill={current ? "var(--glow)" : "var(--surface)"} />
    </g>
  );
}

export function Modal({ title, subtitle, children, footer, onClose, wide }: { title: string; subtitle?: ReactNode; children: ReactNode; footer: ReactNode; onClose: () => void; wide?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    ref.current?.querySelector<HTMLElement>("textarea, input, button.primary")?.focus();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      prev?.focus?.();
    };
  }, [onClose]);
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`dialog${wide ? " wide" : ""}`} role="dialog" aria-modal="true" aria-label={title} ref={ref}>
        <header>
          <h2>{title}</h2>
          {subtitle && <p>{subtitle}</p>}
        </header>
        <div className="body">{children}</div>
        <footer>{footer}</footer>
      </div>
    </div>
  );
}

export function Toasts() {
  const { t } = useTranslation();
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismiss);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {toasts.map((x) => (
        <div key={x.id} className={`toast ${x.kind}`}>
          <span>{x.text}</span>
          <button className="btn ghost small" onClick={() => dismiss(x.id)} aria-label={t("ui.close")}>
            <Icon name="close" size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}

export function Cover({ row, size = "list" }: { row: Pick<GameRow, "coverThumb" | "name" | "nameZh">; size?: "list" | "note" }) {
  const src = imageSrc(row.coverThumb);
  const label = row.nameZh ?? row.name;
  if (!src) return <div className="cover placeholder" aria-hidden>{label.slice(0, 1)}</div>;
  return <img className="cover" src={src} alt="" loading="lazy" data-size={size} />;
}

export function StatusBadge({ row }: { row: Pick<GameRow, "state" | "unconfirmed"> }) {
  const { t } = useTranslation();
  if (row.unconfirmed > 0 && row.state !== "has-changes") return <span className="badge pending">{t("ui.state.pending")}</span>;
  return <span className={`badge ${row.state}`}>{t(`ui.state.${row.state}`)}</span>;
}

export function displayName(row: { name: string; nameZh?: string | null }, lang: string): string {
  return lang.startsWith("zh") && row.nameZh ? row.nameZh : row.name;
}
