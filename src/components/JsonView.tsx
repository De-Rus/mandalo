import { tokenizeLines, type Token } from "../lib/json";

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

interface JsonViewProps {
  text: string;
  highlight: boolean;
  lineNumbers: boolean;
  wrap: boolean;
  query: string;
  current: number;
}

export function JsonView({
  text,
  highlight,
  lineNumbers,
  wrap,
  query,
  current,
}: JsonViewProps) {
  const usable = highlight && text.length <= HIGHLIGHT_LIMIT;
  const lines = usable
    ? tokenizeLines(text)
    : text.split("\n").map((line) => [{ type: "text" as const, text: line }]);
  const cursor: Cursor = { seen: 0 };

  return (
    <div className={`code-view ${wrap ? "code-view-wrap" : ""}`}>
      {lineNumbers && (
        <div className="code-view-gutter" aria-hidden="true">
          {lines.map((_, i) => (
            <div key={i}>{i + 1}</div>
          ))}
        </div>
      )}
      <div className="code-view-body">
        {lines.map((tokens, i) => (
          <div key={i}>
            {tokens.length === 0 ? "​" : renderLine(tokens, query, current, cursor)}
          </div>
        ))}
      </div>
    </div>
  );
}
