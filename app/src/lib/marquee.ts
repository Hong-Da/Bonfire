import { useCallback, useEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type RefObject } from "react";

/** 框选矩形（相对滚动容器的内容坐标）。 */
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

const THRESHOLD = 5;
const EDGE = 40;

/**
 * 按住鼠标拖动框选（U-11）：在滚动容器中拖出矩形，选中与之相交的 `[data-key]` 元素。
 * 按住 Ctrl / Shift 时加到已有的选择上，否则替换。拖到容器上下边缘时自动滚动。
 * 拖动超过几个像素才算框选，此时松开鼠标不会触发点击（不会打开大图）。
 */
export function useMarquee(container: RefObject<HTMLElement | null>, picked: Set<string>, setPicked: (s: Set<string>) => void) {
  const [box, setBox] = useState<Box | null>(null);
  const state = useRef<{ x: number; y: number; base: Set<string>; additive: boolean; dragging: boolean; px: number; py: number } | null>(null);
  const scrollTimer = useRef<number | null>(null);

  const contentPoint = useCallback(
    (cx: number, cy: number) => {
      const el = container.current!;
      const r = el.getBoundingClientRect();
      return { x: cx - r.left + el.scrollLeft, y: cy - r.top + el.scrollTop };
    },
    [container],
  );

  const update = useCallback(() => {
    const s = state.current;
    const el = container.current;
    if (!s || !el || !s.dragging) return;
    const p = contentPoint(s.px, s.py);
    const b = { x: Math.min(s.x, p.x), y: Math.min(s.y, p.y), w: Math.abs(p.x - s.x), h: Math.abs(p.y - s.y) };
    setBox(b);
    const cr = el.getBoundingClientRect();
    const hit = new Set(s.additive ? s.base : []);
    el.querySelectorAll<HTMLElement>("[data-key]").forEach((item) => {
      const r = item.getBoundingClientRect();
      const ix = r.left - cr.left + el.scrollLeft;
      const iy = r.top - cr.top + el.scrollTop;
      if (ix < b.x + b.w && ix + r.width > b.x && iy < b.y + b.h && iy + r.height > b.y) hit.add(item.dataset.key!);
    });
    setPicked(hit);
  }, [container, contentPoint, setPicked]);

  const stop = useCallback(() => {
    if (scrollTimer.current !== null) window.clearInterval(scrollTimer.current);
    scrollTimer.current = null;
  }, []);

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const s = state.current;
      const el = container.current;
      if (!s || !el) return;
      s.px = e.clientX;
      s.py = e.clientY;
      if (!s.dragging) {
        const p = contentPoint(e.clientX, e.clientY);
        if (Math.hypot(p.x - s.x, p.y - s.y) < THRESHOLD) return;
        s.dragging = true;
      }
      e.preventDefault();
      update();
      // 靠近上下边缘时自动滚动。
      const r = el.getBoundingClientRect();
      const dy = e.clientY < r.top + EDGE ? -16 : e.clientY > r.bottom - EDGE ? 16 : 0;
      stop();
      if (dy !== 0) {
        scrollTimer.current = window.setInterval(() => {
          el.scrollTop += dy;
          update();
        }, 30);
      }
    };
    const onUp = () => {
      const s = state.current;
      state.current = null;
      stop();
      setBox(null);
      if (s?.dragging) {
        // 吞掉这次松开鼠标产生的点击。
        const swallow = (ev: Event) => {
          ev.stopPropagation();
          ev.preventDefault();
        };
        window.addEventListener("click", swallow, { capture: true, once: true });
        window.setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
      }
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      stop();
    };
  }, [container, contentPoint, update, stop]);

  const onMouseDown = (e: ReactMouseEvent) => {
    const el = container.current;
    if (e.button !== 0 || !el) return;
    const target = e.target as HTMLElement;
    // 复选框、按钮之外的控件与滚动条上不开始框选。
    if (target.closest("input, select, a, label, .no-marquee")) return;
    if (e.clientX >= el.getBoundingClientRect().left + el.clientWidth) return;
    const p = contentPoint(e.clientX, e.clientY);
    state.current = { ...p, base: new Set(picked), additive: e.ctrlKey || e.shiftKey || e.metaKey, dragging: false, px: e.clientX, py: e.clientY };
  };

  return { box, onMouseDown };
}
