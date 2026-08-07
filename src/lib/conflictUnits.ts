/** Light ###-segment split for conflict UI — no full HttpDoc parse. */

export interface RequestUnit {
  key: string;
  name: string;
  method: string | null;
  detail: string | null;
  text: string;
}

const METHODS = new Set([
  "GET",
  "POST",
  "PUT",
  "PATCH",
  "DELETE",
  "HEAD",
  "OPTIONS",
]);

export function isRequestConflictPath(path: string): boolean {
  return /\.(http|rest|grpc|ws|mqtt)$/i.test(path);
}

export function splitRequestUnits(source: string): RequestUnit[] {
  const normalized = source.replace(/\r\n/g, "\n");
  if (!normalized.trim()) return [];

  const parts = normalized.split(/(?=^### )/m).filter((p) => p.trim().length > 0);
  const chunks = parts.length > 0 ? parts : [normalized];

  return chunks.map((raw, i) => {
    const text = raw.replace(/^\n+/, "").replace(/\n+$/, "") + "\n";
    let name = `Request ${i + 1}`;
    let method: string | null = null;
    let detail: string | null = null;
    for (const line of text.split("\n")) {
      const trimmed = line.trim();
      if (trimmed.startsWith("###")) {
        const n = trimmed.slice(3).trim();
        if (n) name = n;
        continue;
      }
      const m = /^([A-Za-z]+)\s+(\S+)/.exec(trimmed);
      if (m && METHODS.has(m[1].toUpperCase())) {
        method = m[1].toUpperCase();
        detail = m[2];
        break;
      }
      if (/^(wss?:|https?:|mqtts?:)/i.test(trimmed)) {
        detail = trimmed;
        break;
      }
    }
    return {
      key: `${name.toLowerCase()}::${i}`,
      name,
      method,
      detail,
      text,
    };
  });
}

export type UnitKind = "same" | "changed" | "oursOnly" | "theirsOnly";

export interface MatchedUnit {
  id: string;
  kind: UnitKind;
  name: string;
  ours: RequestUnit | null;
  theirs: RequestUnit | null;
}

/** Pair by ### name; leftovers stay as addable one-sided units. */
export function matchRequestUnits(
  oursText: string,
  theirsText: string,
): MatchedUnit[] {
  const ours = splitRequestUnits(oursText);
  const theirs = splitRequestUnits(theirsText);
  const used = new Set<number>();
  const out: MatchedUnit[] = [];
  let n = 0;

  for (const o of ours) {
    const ti = theirs.findIndex(
      (t, i) => !used.has(i) && t.name.toLowerCase() === o.name.toLowerCase(),
    );
    if (ti >= 0) {
      used.add(ti);
      const t = theirs[ti];
      out.push({
        id: `m-${n++}`,
        kind: o.text === t.text ? "same" : "changed",
        name: o.name,
        ours: o,
        theirs: t,
      });
    } else {
      out.push({
        id: `o-${n++}`,
        kind: "oursOnly",
        name: o.name,
        ours: o,
        theirs: null,
      });
    }
  }
  for (let i = 0; i < theirs.length; i++) {
    if (used.has(i)) continue;
    const t = theirs[i];
    out.push({
      id: `t-${n++}`,
      kind: "theirsOnly",
      name: t.name,
      ours: null,
      theirs: t,
    });
  }
  return out;
}

export type UnitPick = "ours" | "theirs" | "both" | "skip";

export function defaultPick(kind: UnitKind): UnitPick {
  switch (kind) {
    case "same":
      return "ours";
    case "changed":
      return "ours";
    case "oursOnly":
      return "ours";
    case "theirsOnly":
      return "theirs";
  }
}

/** Build the merged .http body from per-request picks. */
export function buildRequestMerge(
  units: MatchedUnit[],
  picks: Record<string, UnitPick>,
): string {
  const parts: string[] = [];
  for (const unit of units) {
    const pick = picks[unit.id] ?? defaultPick(unit.kind);
    if (pick === "skip") continue;
    if (pick === "ours" || pick === "both") {
      if (unit.ours) parts.push(unit.ours.text.trimEnd());
    }
    if (pick === "theirs" || pick === "both") {
      if (unit.theirs) {
        let text = unit.theirs.text.trimEnd();
        if (pick === "both" && unit.ours && unit.ours.name === unit.theirs.name) {
          text = text.replace(/^###\s+[^\n]*/m, `### ${unit.theirs.name} (remote)`);
        }
        parts.push(text);
      }
    }
  }
  if (parts.length === 0) return "";
  return parts.join("\n\n") + "\n";
}
