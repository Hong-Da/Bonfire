import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { imageSrc } from "../api";
import { useStore } from "../store";
import type { GameRow } from "../types";

export type IconName =
  | "library" | "settings" | "scan" | "backup" | "restore" | "branch" | "tag" | "search" | "folder"
  | "close" | "plus" | "undo" | "cloud" | "edit" | "trash" | "check" | "export" | "import" | "more" | "game" | "chevron" | "next" | "shield" | "camera" | "info" | "expand" | "image" | "clock" | "stack" | "gallery" | "filter";

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
  export: "M12 15V4m0 0L8 8m4-4 4 4M5 14v5h14v-5",
  import: "M12 4v11m0 0-4-4m4 4 4-4M5 14v5h14v-5",
  more: "M6 12h.01M12 12h.01M18 12h.01",
  game: "M7 8h10a4 4 0 0 1 4 4v1a3 3 0 0 1-5.4 1.8L14.5 13h-5l-1.1 1.8A3 3 0 0 1 3 13v-1a4 4 0 0 1 4-4zM8 10v3M6.5 11.5h3M16 11h.01M18 12.5h.01",
  chevron: "M8 10l4 4 4-4",
  next: "M10 7l5 5-5 5",
  camera: "M4 8h3l2-3h6l2 3h3v11H4zM12 16.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z",
  shield: "M12 3l7 3v5c0 4.6-3 8.4-7 10-4-1.6-7-5.4-7-10V6zM9 12l2 2 4-4",
  info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v5M12 7.5v.5",
  expand: "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5",
  image: "M4 5h16v14H4zM4 15l4-4 4 4 3-3 5 5M15 9.5a1.5 1.5 0 1 0 0-.01",
  clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2",
  stack: "M12 4l8 4-8 4-8-4zM4 12l8 4 8-4M4 16l8 4 8-4",
  gallery: "M3 7h4v4H3zM10 7h4v4h-4zM17 7h4v4h-4zM3 14h4v4H3zM10 14h4v4h-4zM17 14h4v4h-4z",
  filter: "M4 5h16l-6 7.5V18l-4 2v-7.5z",
};

export function Icon({ name, size = 18, label }: { name: IconName; size?: number; label?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <path d={PATHS[name]} />
    </svg>
  );
}

/** 时间线上的篝火：快照节点。当前所在的快照发光；自动生成的快照用所在路线的颜色。 */
export function Flame({ x, y, current, auto, color }: { x: number; y: number; current?: boolean; auto?: boolean; color?: string }) {
  const fill = auto ? (color ?? "var(--lane-0)") : "var(--ember)";
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

/** 下拉菜单：点按钮展开，点外部或按 Esc 关闭。 */
export interface MenuItem {
  label: ReactNode;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
  hint?: string;
  /** 单选项：显示勾选标记。 */
  checked?: boolean;
}

export function Menu({ label, icon, items, className = "btn", align = "left", title }: {
  label?: ReactNode;
  icon?: IconName;
  items: (MenuItem | "divider")[];
  className?: string;
  align?: "left" | "right";
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div className="menu" ref={ref}>
      <button className={className} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)} title={title} aria-label={title}>
        {icon && <Icon name={icon} />} {label}
        {label && <Icon name="chevron" size={14} />}
      </button>
      {open && (
        <div className={`menu-list ${align}`} role="menu">
          {items.map((it, i) =>
            it === "divider" ? (
              <hr key={i} />
            ) : (
              <button
                key={i}
                role={it.checked === undefined ? "menuitem" : "menuitemradio"}
                aria-checked={it.checked}
                className={[it.danger && "danger", it.checked !== undefined && "checkable"].filter(Boolean).join(" ") || undefined}
                disabled={it.disabled}
                title={it.hint}
                onClick={() => {
                  setOpen(false);
                  it.onClick();
                }}
              >
                {it.checked !== undefined && <span className="mark">{it.checked && <Icon name="check" size={14} />}</span>}
                {it.label}
              </button>
            ),
          )}
        </div>
      )}
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
          {x.action && (
            <button
              className="btn small"
              onClick={() => {
                dismiss(x.id);
                x.action?.run();
              }}
            >
              {x.action.label}
            </button>
          )}
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
