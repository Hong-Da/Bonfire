import { useTranslation } from "react-i18next";

/**
 * 底栏的全选框与反选（游戏库与图库共用）：全选框勾选 / 取消列出的全部项，部分勾选时显示半选。
 */
export default function SelectSeg({ total, picked, onAll, onNone, onInvert, label, hint, compact, checkLabel }: {
  /** 列出的项数 */
  total: number;
  /** 其中已勾选的项数 */
  picked: number;
  onAll: () => void;
  onNone: () => void;
  onInvert: () => void;
  label: string;
  hint: string;
  /** 小号（图库分组标题中用） */
  compact?: boolean;
  /** 全选框的无障碍名称（默认为“全选”） */
  checkLabel?: string;
}) {
  const { t } = useTranslation();
  const all = total > 0 && picked === total;
  return (
    <div className={`seg sel-seg${compact ? " compact" : ""}`} role="group" aria-label={label}>
      <label className="sel-all" title={hint}>
        <input
          type="checkbox"
          className="check"
          checked={all}
          disabled={total === 0}
          ref={(el) => {
            if (el) el.indeterminate = picked > 0 && !all;
          }}
          onChange={() => (all ? onNone() : onAll())}
          aria-label={checkLabel}
        />
        <span>{t("ui.library.selectAll")}</span>
      </label>
      <button onClick={onInvert} disabled={total === 0} title={t("ui.select.invertHint")}>
        {t("ui.library.invert")}
      </button>
    </div>
  );
}
