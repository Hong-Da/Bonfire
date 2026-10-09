import { useEffect, useState } from "react";

/**
 * 忙碌状态持续超过 `delay` 毫秒才返回 true。
 * 很快完成的操作（如检查更新）不显示进度，避免按钮文字、取消按钮和进度条一闪而过。
 */
export function useDelayed(busy: boolean, delay = 400): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!busy) {
      setShown(false);
      return;
    }
    const t = setTimeout(() => setShown(true), delay);
    return () => clearTimeout(t);
  }, [busy, delay]);
  return busy && shown;
}
