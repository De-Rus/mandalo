import { copyText, formatClock, hexPreview, type LogRow } from "../../lib/streamLog";
import { formatBytes } from "../../lib/format";
import { toast } from "../../store/toast";
import { ChevronDown, ChevronRight, Copy } from "../Icons";

export const COLLAPSED_HEIGHT = 26;
const LINE_HEIGHT = 17;
const MAX_LINES = 24;
const DETAIL_PADDING = 16;

export function expandedLines(row: LogRow): number {
  const text = row.pretty ?? row.detail ?? row.summary;
  return Math.min(text.split("\n").length, MAX_LINES);
}

export function rowHeight(row: LogRow, expanded: boolean): number {
  if (!expanded || !row.expandable) return COLLAPSED_HEIGHT;
  return COLLAPSED_HEIGHT + expandedLines(row) * LINE_HEIGHT + DETAIL_PADDING;
}

interface Props {
  row: LogRow;
  expanded: boolean;
  onToggle: () => void;
}

/**
 * Raw bytes never reach the DOM as bytes: a binary frame shows its size and a
 * hex window, which is readable and cannot smuggle control characters into the
 * layout.
 */
function BinaryDetail({ row }: { row: LogRow }) {
  if (row.payload?.kind !== "binary") return null;
  return (
    <div className="log-binary">
      <div className="log-binary-head">
        {formatBytes(row.payload.bytes)} of binary
      </div>
      <pre className="log-detail-body mono">{hexPreview(row.payload.base64, 256)}</pre>
    </div>
  );
}

export function StreamLogRow({ row, expanded, onToggle }: Props) {
  const open = expanded && row.expandable;
  return (
    <div
      className={`log-row log-${row.tone} ${open ? "log-row-open" : ""}`}
      data-seq={row.seq}
      style={{ height: rowHeight(row, expanded) }}
    >
      <div
        className="log-line"
        role={row.expandable ? "button" : undefined}
        tabIndex={row.expandable ? 0 : undefined}
        aria-expanded={row.expandable ? open : undefined}
        onClick={row.expandable ? onToggle : undefined}
        onKeyDown={(e) => {
          if (!row.expandable) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onToggle();
          }
        }}
      >
        <span className="log-caret">
          {row.expandable ? (
            open ? (
              <ChevronDown size={10} />
            ) : (
              <ChevronRight size={10} />
            )
          ) : null}
        </span>
        <span className="log-clock mono">{formatClock(row.at)}</span>
        <span className={`log-gutter log-gutter-${row.tone}`}>{row.gutter}</span>
        <span className="log-summary mono">{row.summary}</span>
        <span className="log-chips">
          {row.chips.map((chip) => (
            <span className="log-chip" key={chip}>
              {chip}
            </span>
          ))}
        </span>
        <button
          className="btn-ghost btn-icon log-copy"
          aria-label={`Copy message ${row.seq + 1}`}
          title="Copy"
          onClick={(e) => {
            e.stopPropagation();
            void navigator.clipboard.writeText(copyText(row));
            toast("success", "Message copied");
          }}
        >
          <Copy size={12} />
        </button>
      </div>
      {open && (
        <div className="log-detail" style={{ maxHeight: expandedLines(row) * LINE_HEIGHT }}>
          {row.payload?.kind === "binary" ? (
            <BinaryDetail row={row} />
          ) : (
            <pre className="log-detail-body mono">{row.pretty ?? row.detail}</pre>
          )}
        </div>
      )}
    </div>
  );
}
