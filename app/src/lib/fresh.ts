import { prefGet, prefSet } from "./uiState";

/**
 * 新加入游戏库的游戏（U-34）：记录每个游戏在本机第一次出现在游戏库中的时间，
 * 7 天内、且还没打开过详情的标为“新加入”。第一次使用时已有的游戏不算新加入。
 * 只是界面提示，记在本机；读写失败时不标记。
 */
const KEY = "bonfire.library.seen";
export const FRESH_DAYS = 7;

/** 游戏 ID → 第一次出现的时间（毫秒）；0 表示不算新加入（已有的或已打开过）。 */
type Seen = Record<string, number>;

function read(): Seen | null {
  try {
    const raw = prefGet(KEY);
    if (raw === null) return null;
    const v = JSON.parse(raw);
    return v && typeof v === "object" ? (v as Seen) : {};
  } catch {
    return {};
  }
}

function write(s: Seen) {
  try {
    prefSet(KEY, JSON.stringify(s));
  } catch {
    // 无法保存时只影响本次。
  }
}

function freshOf(s: Seen, ids: string[], now: number): Record<string, number> {
  const out: Record<string, number> = {};
  for (const id of ids) {
    const at = s[id];
    if (typeof at === "number" && at > 0 && now - at < FRESH_DAYS * 86_400_000) out[id] = at;
  }
  return out;
}

/** 记下游戏库中的游戏，返回新加入的（ID → 加入时间）。 */
export function noteGames(ids: string[], now = Date.now()): Record<string, number> {
  const stored = read();
  const first = stored === null;
  const s: Seen = stored ?? {};
  let changed = first;
  for (const id of ids) {
    if (!(id in s)) {
      s[id] = first ? 0 : now;
      changed = true;
    }
  }
  if (changed) write(s);
  return freshOf(s, ids, now);
}

/** 打开过详情或手动标为已看过：不再算新加入。 */
export function markSeen(ids: string[]) {
  const s = read() ?? {};
  for (const id of ids) s[id] = 0;
  write(s);
}
