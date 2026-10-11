import { useTranslation } from "react-i18next";
import type { RetentionConfig } from "../types";

/**
 * 历史清理的规则说明与安全提示（设置页与清理对话框共用）。
 * 按实际执行顺序列出：选出可清理的 → 按规则保留 → 永远保留 → 删除其余 → 回收空间。
 */
export function RetentionSteps({ ret, protectLocal }: { ret: RetentionConfig; protectLocal: boolean }) {
  const { t } = useTranslation();
  return (
    <ol className="retention-steps">
      <li>
        <b>{t("ui.retentionGuide.s1")}</b>
        <span>{ret.scope === "auto" ? t("ui.retentionGuide.s1auto") : protectLocal ? t("ui.retentionGuide.s1allProtected") : t("ui.retentionGuide.s1all")}</span>
      </li>
      <li>
        <b>{t("ui.retentionGuide.s2")}</b>
        <span>{t("ui.retentionGuide.s2body", { last: ret.keep_last, days: ret.keep_daily, weeks: ret.keep_weekly })}</span>
      </li>
      <li>
        <b>{t("ui.retentionGuide.s3")}</b>
        <span>{t("ui.retentionGuide.s3body")}</span>
      </li>
      <li>
        <b>{t("ui.retentionGuide.s4")}</b>
        <span>{t("ui.retentionGuide.s4body")}</span>
      </li>
      <li>
        <b>{t("ui.retentionGuide.s5")}</b>
        <span>{t("ui.retentionGuide.s5body")}</span>
      </li>
    </ol>
  );
}

export function RetentionSafety({ auto }: { auto?: boolean }) {
  const { t } = useTranslation();
  return (
    <ul className="retention-safety">
      <li>{t("ui.retentionGuide.safe1")}</li>
      <li>{t("ui.retentionGuide.safe2")}</li>
      {auto && <li>{t("ui.retentionGuide.safe3")}</li>}
      <li>{t("ui.retentionGuide.safe4")}</li>
    </ul>
  );
}

/** 设置页中的完整说明：规则、示例与安全提示，默认展开。 */
export function RetentionGuide({ ret, protectLocal }: { ret: RetentionConfig; protectLocal: boolean }) {
  const { t } = useTranslation();
  const most = ret.keep_last + ret.keep_daily + ret.keep_weekly;
  return (
    <details className="retention-guide" open>
      <summary>{t("ui.retentionGuide.title")}</summary>
      <RetentionSteps ret={ret} protectLocal={protectLocal} />
      <p className="retention-example">{t("ui.retentionGuide.example", { last: ret.keep_last, days: ret.keep_daily, weeks: ret.keep_weekly, most })}</p>
      <div className="retention-safety-box" role="note">
        <b>{t("ui.retentionGuide.safeTitle")}</b>
        <RetentionSafety auto />
      </div>
    </details>
  );
}
