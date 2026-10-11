import { useTranslation } from "react-i18next";
import { bytes } from "../lib/format";
import type { GcSource } from "../types";

/** 可释放空间的来源明细：每个来源的大小与说明，按大小占比画一条细条。 */
export function SpaceSources({ sources, total }: { sources: GcSource[]; total: number }) {
  const { t } = useTranslation();
  if (sources.length === 0 || total === 0) return null;
  return (
    <section className="space-sources">
      <h3>{t("ui.space.title")}</h3>
      <ul>
        {sources.map((s) => (
          <li key={s.kind}>
            <div className="ss-head">
              <span>{t(`ui.space.kind.${s.kind}`)}</span>
              <b className="num">{bytes(s.bytes)}</b>
            </div>
            <div className="ss-bar" aria-hidden>
              <i style={{ width: `${Math.max(2, (s.bytes / total) * 100)}%` }} />
            </div>
            <small>{t(`ui.space.hint.${s.kind}`)}</small>
          </li>
        ))}
      </ul>
    </section>
  );
}
