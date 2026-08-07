/** Line-oriented diff: every inserted, removed, or equal line. */
export type DiffOp = "equal" | "insert" | "delete";

export interface DiffLine {
  op: DiffOp;
  text: string;
  left?: number;
  right?: number;
}

/**
 * Myers-inspired O(ND) line diff. Small Mandalo files only — fine for conflict UI.
 */
export function lineDiff(left: string, right: string): DiffLine[] {
  const a = left.split("\n");
  const b = right.split("\n");
  // Drop a single trailing empty from split so "a\n" vs "a" stays honest.
  if (a.length > 0 && a[a.length - 1] === "") a.pop();
  if (b.length > 0 && b[b.length - 1] === "") b.pop();

  const n = a.length;
  const m = b.length;
  const max = n + m;
  const offset = max;
  const v = new Array<number>(2 * max + 1).fill(0);
  const trace: number[][] = [];

  for (let d = 0; d <= max; d++) {
    const snap = v.slice();
    trace.push(snap);
    for (let k = -d; k <= d; k += 2) {
      let x: number;
      if (k === -d || (k !== d && v[k - 1 + offset] < v[k + 1 + offset])) {
        x = v[k + 1 + offset];
      } else {
        x = v[k - 1 + offset] + 1;
      }
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[k + offset] = x;
      if (x >= n && y >= m) {
        return backtrack(trace, a, b, offset);
      }
    }
  }
  return [];
}

function backtrack(
  trace: number[][],
  a: string[],
  b: string[],
  offset: number,
): DiffLine[] {
  const out: DiffLine[] = [];
  let x = a.length;
  let y = b.length;
  for (let d = trace.length - 1; d >= 0; d--) {
    const v = trace[d];
    const k = x - y;
    let prevK: number;
    if (k === -d || (k !== d && v[k - 1 + offset] < v[k + 1 + offset])) {
      prevK = k + 1;
    } else {
      prevK = k - 1;
    }
    const prevX = v[prevK + offset];
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      out.push({ op: "equal", text: a[x - 1], left: x, right: y });
      x--;
      y--;
    }
    if (d === 0) break;
    if (x > prevX) {
      out.push({ op: "delete", text: a[x - 1], left: x });
      x--;
    } else if (y > prevY) {
      out.push({ op: "insert", text: b[y - 1], right: y });
      y--;
    }
  }
  out.reverse();
  return out;
}
