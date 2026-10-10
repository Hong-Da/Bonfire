import { pinyin } from "pinyin-pro";

/**
 * 游戏名的模糊搜索（U-18）：中文名、英文名都能搜；不区分大小写，忽略空格与标点、重音；
 * 中文名还可以用拼音或拼音首字母搜（“kongdong”“kdqs” → 空洞骑士）；
 * 字母按顺序出现即可（“hk” → Hollow Knight）。多个关键词用空格分开，需全部命中。
 */

const PUNCT = /[\s\-_:：·・'’"“”.,，。!！?？()（）[\]【】/\\&+~]/g;

/** 规范化：小写、去掉重音、空格与标点。 */
export function normalize(s: string): string {
  return s.normalize("NFKD").replace(/[̀-ͯ]/g, "").toLowerCase().replace(PUNCT, "");
}

const HAN = /[㐀-鿿]/;
const cache = new Map<string, string[]>();

/** 一个名称可供匹配的形式：原文，含中文时再加全拼与拼音首字母（结果缓存）。 */
function forms(name: string): string[] {
  let f = cache.get(name);
  if (f) return f;
  f = [normalize(name)];
  if (HAN.test(name)) {
    f.push(normalize(pinyin(name, { toneType: "none", type: "array", nonZh: "consecutive" }).join("")));
    f.push(normalize(pinyin(name, { pattern: "first", toneType: "none", type: "array", nonZh: "consecutive" }).join("")));
  }
  if (cache.size > 5000) cache.clear();
  cache.set(name, f);
  return f;
}

/** `needle` 的字符按顺序出现在 `hay` 中。 */
function subsequence(needle: string, hay: string): boolean {
  let i = 0;
  for (const c of hay) {
    if (c === needle[i]) i += 1;
    if (i === needle.length) return true;
  }
  return needle.length === 0;
}

/** 关键词是否命中这些名称（任一名称的任一形式）。 */
export function fuzzyMatch(query: string, names: (string | null | undefined)[]): boolean {
  const tokens = query.split(/\s+/).map(normalize).filter(Boolean);
  if (tokens.length === 0) return true;
  const all = names.filter((n): n is string => !!n).flatMap(forms);
  return tokens.every((tok) =>
    all.some((f) => f.includes(tok) || (tok.length >= 2 && !HAN.test(tok) && subsequence(tok, f))),
  );
}
