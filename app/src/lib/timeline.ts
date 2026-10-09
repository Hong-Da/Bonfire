/**
 * 历史图布局：把按时间排列（新→旧）的快照分配到若干“路线”（列），
 * 同一分支的快照留在同一列，分叉处画斜线。存档不能合并，所以每个快照最多一个父快照在图中。
 */

export interface GraphInput {
  id: string;
  parents: string[];
}

export interface GraphRow {
  id: string;
  row: number;
  col: number;
}

export interface GraphEdge {
  fromRow: number;
  fromCol: number;
  toRow: number;
  toCol: number;
}

export interface GraphLayout {
  rows: GraphRow[];
  edges: GraphEdge[];
  columns: number;
}

export function layout(nodes: GraphInput[]): GraphLayout {
  // lanes[i] = 这一列接下来等待的快照 ID（即上方某个快照的父快照）。
  const lanes: (string | null)[] = [];
  const pos = new Map<string, GraphRow>();
  const rows: GraphRow[] = [];
  const known = new Set(nodes.map((n) => n.id));

  nodes.forEach((n, row) => {
    const waiting = lanes.map((id, i) => (id === n.id ? i : -1)).filter((i) => i >= 0);
    let col: number;
    if (waiting.length > 0) {
      col = waiting[0];
      // 其他等待同一快照的列在这里汇合（即从这里分叉出去），释放。
      for (const i of waiting.slice(1)) lanes[i] = null;
    } else {
      const free = lanes.indexOf(null);
      col = free >= 0 ? free : lanes.length;
    }
    const parent = n.parents.find((p) => known.has(p)) ?? null;
    lanes[col] = parent;
    // 去掉末尾的空列，让图保持紧凑。
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
    const r = { id: n.id, row, col };
    pos.set(n.id, r);
    rows.push(r);
  });

  const edges: GraphEdge[] = [];
  for (const n of nodes) {
    const from = pos.get(n.id)!;
    for (const p of n.parents) {
      const to = pos.get(p);
      if (to) edges.push({ fromRow: from.row, fromCol: from.col, toRow: to.row, toCol: to.col });
    }
  }
  const columns = rows.reduce((m, r) => Math.max(m, r.col + 1), 1);
  return { rows, edges, columns };
}

/** 从若干快照出发能到达的全部快照（含自身），只在 `nodes` 内查找。 */
export function ancestors(nodes: GraphInput[], starts: string[]): Set<string> {
  const parents = new Map(nodes.map((n) => [n.id, n.parents]));
  const seen = new Set<string>();
  const stack = starts.filter((s) => parents.has(s));
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    for (const p of parents.get(id) ?? []) if (parents.has(p)) stack.push(p);
  }
  return seen;
}

/** 一段历史：`to` 之前（含）的快照，去掉 `from` 的父快照之前的部分；`from` 为空时到最早。与后端 BundleScope::Range 一致。 */
export function rangeOf(nodes: GraphInput[], from: string | null, to: string): Set<string> {
  const set = ancestors(nodes, [to]);
  if (from) {
    const parents = nodes.find((n) => n.id === from)?.parents ?? [];
    for (const id of ancestors(nodes, parents)) set.delete(id);
  }
  return set;
}
