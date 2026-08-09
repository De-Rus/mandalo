import { useEffect, useState } from "react";
import { leafPaths, tokenizeLines, type JsonLeaf, type Token } from "../lib/json";
import { Branch, Check, ChevronDown, ChevronRight, Copy } from "./Icons";

export type LeafAction = "copy" | "capture" | "assert";

const HIGHLIGHT_LIMIT = 300_000;

export function countMatches(text: string, query: string): number {
  if (query === "") return 0;
  let count = 0;
  let index = text.toLowerCase().indexOf(query.toLowerCase());
  while (index !== -1) {
    count++;
    index = text.toLowerCase().indexOf(query.toLowerCase(), index + query.length);
  }
  return count;
}

interface Cursor {
  seen: number;
}

function marked(
  text: string,
  query: string,
  current: number,
  cursor: Cursor,
): React.ReactNode {
  if (query === "") return text;
  const parts: React.ReactNode[] = [];
  const lower = text.toLowerCase();
  const needle = query.toLowerCase();
  let from = 0;
  let index = lower.indexOf(needle);
  while (index !== -1) {
    if (index > from) parts.push(text.slice(from, index));
    const isCurrent = cursor.seen === current;
    parts.push(
      <mark key={`${index}-${cursor.seen}`} className={isCurrent ? "mark-current" : ""}>
        {text.slice(index, index + query.length)}
      </mark>,
    );
    cursor.seen++;
    from = index + query.length;
    index = lower.indexOf(needle, from);
  }
  if (from < text.length) parts.push(text.slice(from));
  return parts;
}

function renderLine(
  tokens: Token[],
  query: string,
  current: number,
  cursor: Cursor,
): React.ReactNode {
  return tokens.map((token, i) => (
    <span key={i} className={`tk-${token.type}`}>
      {marked(token.text, query, current, cursor)}
    </span>
  ));
}

function lineText(tokens: Token[]): string {
  return tokens.map((t) => t.text).join("");
}

function foldRanges(lines: Token[][]): Map<number, number> {
  const ranges = new Map<number, number>();
  const open: number[] = [];
  lines.forEach((tokens, i) => {
    for (const token of tokens) {
      if (token.type !== "punct") continue;
      for (const char of token.text) {
        if (char === "{" || char === "[") open.push(i);
        else if (char === "}" || char === "]") {
          const start = open.pop();
          if (start !== undefined && start < i) ranges.set(start, i);
        }
      }
    }
  });
  return ranges;
}

function hiddenLines(
  ranges: Map<number, number>,
  folded: ReadonlySet<number>,
): Set<number> {
  const hidden = new Set<number>();
  for (const start of folded) {
    const end = ranges.get(start);
    if (end === undefined) continue;
    for (let i = start + 1; i <= end; i++) hidden.add(i);
  }
  return hidden;
}

const ACTION_ICONS: Record<LeafAction, React.ReactNode> = {
  copy: <Copy size={10} />,
  capture: <Branch size={10} />,
  assert: <Check size={10} />,
};

function actionLabel(action: LeafAction, source: string): string {
  switch (action) {
    case "copy":
      return `Copy path ${source}`;
    case "capture":
      return `Capture ${source} into a variable`;
    case "assert":
      return `Assert on ${source}`;
  }
}

interface JsonViewProps {
  text: string;
  highlight: boolean;
  lineNumbers: boolean;
  wrap: boolean;
  query: string;
  current: number;
  actions?: LeafAction[];
  onAction?: (action: LeafAction, leaf: JsonLeaf) => void;
}

export function JsonView({
  text,
  highlight,
  lineNumbers,
  wrap,
  query,
  current,
  actions = [],
  onAction,
}: JsonViewProps) {
  const [folded, setFolded] = useState<ReadonlySet<number>>(new Set());

  useEffect(() => setFolded(new Set()), [text]);

  const usable = highlight && text.length <= HIGHLIGHT_LIMIT;
  const lines = usable
    ? tokenizeLines(text)
    : text.split("\n").map((line) => [{ type: "text" as const, text: line }]);
  const ranges = usable ? foldRanges(lines) : new Map<number, number>();
  const offered = onAction ? actions : [];
  const leaves =
    usable && offered.length > 0 ? leafPaths(lines) : lines.map(() => null);
  const hidden = hiddenLines(ranges, folded);
  const cursor: Cursor = { seen: 0 };

  const numbers: React.ReactNode[] = [];
  const arrows: React.ReactNode[] = [];
  const rows: React.ReactNode[] = [];

  lines.forEach((tokens, i) => {
    if (hidden.has(i)) {
      cursor.seen += countMatches(lineText(tokens), query);
      return;
    }
    numbers.push(<div key={i}>{i + 1}</div>);
    const end = ranges.get(i);
    const shut = folded.has(i);
    arrows.push(
      end === undefined ? (
        <div key={i} className="code-fold-blank" />
      ) : (
        <button
          key={i}
          className="code-fold"
          aria-expanded={!shut}
          aria-label={`${shut ? "Expand" : "Collapse"} lines ${i + 1} to ${end + 1}`}
          onClick={() => {
            const next = new Set(folded);
            if (shut) next.delete(i);
            else next.add(i);
            setFolded(next);
          }}
        >
          {shut ? <ChevronRight size={10} /> : <ChevronDown size={10} />}
        </button>
      ),
    );
    const leaf = leaves[i];
    rows.push(
      <div key={i}>
        {tokens.length === 0 ? "​" : renderLine(tokens, query, current, cursor)}
        {shut && <span className="code-fold-mark">⋯</span>}
        {leaf && onAction && (
          <span className="code-line-actions">
            {offered.map((action) => (
              <button
                key={action}
                className="code-line-act"
                aria-label={actionLabel(action, `body.${leaf.path}`)}
                title={actionLabel(action, `body.${leaf.path}`)}
                onClick={() => onAction(action, leaf)}
              >
                {ACTION_ICONS[action]}
              </button>
            ))}
          </span>
        )}
      </div>,
    );
  });

  return (
    <div className={`code-view ${wrap ? "code-view-wrap" : ""}`}>
      {lineNumbers && (
        <div className="code-view-gutter" aria-hidden="true">
          {numbers}
        </div>
      )}
      {ranges.size > 0 && <div className="code-view-folds">{arrows}</div>}
      <div className="code-view-body">{rows}</div>
    </div>
  );
}
