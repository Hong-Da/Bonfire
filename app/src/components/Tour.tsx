import { useState } from "react";
import { useTranslation } from "react-i18next";
import { prefGet, prefSet } from "../lib/uiState";
import type { View } from "../store";
import { Icon, Modal, type IconName } from "./ui";

/**
 * 功能导览（0.4.0，设计稿 docs/design/wizard-2026-10-11）：8 张卡片介绍常用功能，
 * 每张可“去试试”跳到对应页面。记住看到第几张，下次从那里继续；看过一次后不再自动弹出。
 */
export const TOUR = [
  { key: "scan", icon: "search", go: { name: "scan" } },
  { key: "backup", icon: "backup", go: { name: "library" } },
  { key: "restore", icon: "branch", go: { name: "library" } },
  { key: "auto", icon: "clock", go: { name: "settings", section: "auto" } },
  { key: "gallery", icon: "gallery", go: { name: "gallery" } },
  { key: "sync", icon: "cloud", go: { name: "sync" } },
  { key: "overview", icon: "home", go: { name: "overview" } },
  { key: "share", icon: "export", go: { name: "library" } },
] as const satisfies readonly { key: string; icon: IconName; go: View }[];

const INDEX_KEY = "tour.index";
export const SEEN_KEY = "tour.seen";

export default function Tour({ onClose, onGo }: { onClose: () => void; onGo: (v: View) => void }) {
  const { t } = useTranslation();
  const [i, setI] = useState(() => Math.min(TOUR.length - 1, Math.max(0, Number(prefGet(INDEX_KEY) ?? 0) || 0)));
  const card = TOUR[i];
  const move = (n: number) => {
    setI(n);
    prefSet(INDEX_KEY, String(n));
  };
  const close = () => {
    prefSet(SEEN_KEY, "1");
    // 看完后下次从第一张开始。
    if (i === TOUR.length - 1) prefSet(INDEX_KEY, "0");
    onClose();
  };
  const points = t(`ui.tour.${card.key}.points`).split("\n").filter(Boolean);
  return (
    <Modal title={t("ui.tour.title")} wide onClose={close} footer={
        <div className="tour-foot">
          <div className="tour-dots" role="tablist" aria-label={t("ui.tour.title")}>
            {TOUR.map((c, k) => (
              <button key={c.key} role="tab" aria-selected={k === i} aria-label={t(`ui.tour.${c.key}.title`)} className={k <= i ? "on" : ""} onClick={() => move(k)} />
            ))}
          </div>
          <span className="grow" />
          <button className="btn ghost" disabled={i === 0} onClick={() => move(i - 1)}>
            {t("ui.tour.prev")}
          </button>
          <button
            className="btn"
            onClick={() => {
              prefSet(INDEX_KEY, String(i));
              prefSet(SEEN_KEY, "1");
              onGo(card.go);
              onClose();
            }}
          >
            {t("ui.tour.try")}
          </button>
          {i < TOUR.length - 1 ? (
            <button className="btn primary" onClick={() => move(i + 1)}>
              {t("ui.tour.next")}
            </button>
          ) : (
            <button className="btn primary" onClick={close}>
              {t("ui.tour.done")}
            </button>
          )}
        </div>
      }>
      <div className="tour" aria-live="polite">
        <p className="tour-sub muted">{t("ui.tour.sub", { n: TOUR.length })}</p>
        <div className="tour-card">
          <div className={`tour-scene scene-${card.key}`} aria-hidden>
            <Icon name={card.icon} size={44} />
          </div>
          <div className="tour-text">
            <p className="tour-step num">{t("ui.tour.step", { n: i + 1, total: TOUR.length })}</p>
            <h3>{t(`ui.tour.${card.key}.title`)}</h3>
            <p>{t(`ui.tour.${card.key}.body`)}</p>
            <ul>
              {points.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ul>
          </div>
        </div>
      </div>
    </Modal>
  );
}
