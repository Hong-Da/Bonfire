import { create } from "zustand";
import { useTranslation } from "react-i18next";
import { Icon, Modal, type IconName } from "../components/ui";

/**
 * 应用内的确认对话框（替代系统原生弹窗，与应用界面一致）。
 * `confirm(text, opts)` 返回用户是否确认；文字中的空行分段显示。由 <ConfirmHost /> 统一渲染。
 */
export interface ConfirmOptions {
  title?: string;
  kind?: "info" | "warning";
  /** 确认按钮的文字，默认“确定”。 */
  ok?: string;
  icon?: IconName;
}

interface Pending extends ConfirmOptions {
  text: string;
  resolve: (ok: boolean) => void;
}

const useConfirm = create<{ pending: Pending | null }>(() => ({ pending: null }));

export function confirm(text: string, opts: ConfirmOptions = {}): Promise<boolean> {
  // 同时只显示一个：新的确认到来时，之前未回答的视为取消。
  useConfirm.getState().pending?.resolve(false);
  return new Promise((resolve) => useConfirm.setState({ pending: { ...opts, text, resolve } }));
}

export function ConfirmHost() {
  const { t } = useTranslation();
  const p = useConfirm((s) => s.pending);
  if (!p) return null;
  const answer = (ok: boolean) => {
    useConfirm.setState({ pending: null });
    p.resolve(ok);
  };
  const warning = p.kind === "warning";
  const [first, ...rest] = p.text.split(/\n\s*\n/);
  return (
    <Modal
      title={p.title ?? t("ui.confirm.title")}
      onClose={() => answer(false)}
      footer={
        <>
          <button className="btn ghost" onClick={() => answer(false)}>
            {t("ui.cancel")}
          </button>
          <button className={`btn primary${warning ? " danger" : ""}`} onClick={() => answer(true)}>
            {p.ok ?? t("ui.confirm.ok")}
          </button>
        </>
      }
    >
      <div className={`confirm-body${warning ? " warning" : ""}`}>
        <span className="confirm-icon" aria-hidden>
          <Icon name={p.icon ?? (warning ? "shield" : "info")} size={22} />
        </span>
        <div>
          <p className="confirm-lead">{first}</p>
          {rest.map((para, i) => (
            <p key={i} className="confirm-more">
              {para}
            </p>
          ))}
        </div>
      </div>
    </Modal>
  );
}
