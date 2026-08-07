import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { atBottom } from "../../lib/autoscroll";
import {
  copyText,
  LOG_CAP,
  visibleRows,
  type LogFilter,
  type LogRow,
} from "../../lib/streamLog";
import { toast } from "../../store/toast";
import { ArrowDown, Copy, Inbox, Search, Trash } from "../Icons";
import { rowHeight, StreamLogRow } from "./LogRow";

const OVERSCAN = 6;

const FILTERS: { id: LogFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "incoming", label: "Incoming" },
  { id: "outgoing", label: "Outgoing" },
  { id: "lifecycle", label: "Connection" },
];

function offsetsOf(rows: LogRow[], expanded: Set<number>): number[] {
  const offsets = new Array<number>(rows.length + 1);
  offsets[0] = 0;
  for (let i = 0; i < rows.length; i += 1)
    offsets[i + 1] = offsets[i] + rowHeight(rows[i], expanded.has(rows[i].seq));
  return offsets;
}

function firstAfter(offsets: number[], y: number): number {
  let low = 0;
  let high = offsets.length - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (offsets[mid + 1] <= y) low = mid + 1;
    else high = mid;
  }
  return low;
}

interface Props {
  rows: LogRow[];
  overflow: number;
  onClear: () => void;
}

/**
 * The log is windowed: only the rows in view are in the DOM, so a firehose costs
 * a bounded amount of layout no matter how long it runs. The store keeps the
 * last LOG_CAP rows and says out loud when it has thrown any away.
 */
export function MessageLog({ rows, overflow, onClear }: Props) {
  const [filter, setFilter] = useState<LogFilter>("all");
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [following, setFollowing] = useState(true);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(320);
  const scrollRef = useRef<HTMLDivElement>(null);

  const shown = useMemo(
    () => visibleRows(rows, filter, query),
    [rows, filter, query],
  );
  const offsets = useMemo(() => offsetsOf(shown, expanded), [shown, expanded]);
  const total = offsets[offsets.length - 1] ?? 0;

  const start = Math.max(0, firstAfter(offsets, scrollTop) - OVERSCAN);
  const end = Math.min(
    shown.length,
    firstAfter(offsets, scrollTop + viewport) + 1 + OVERSCAN,
  );
  const window = shown.slice(start, end);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    setScrollTop(el.scrollTop);
    setViewport(el.clientHeight);
    setFollowing(atBottom(el));
  }, []);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    setViewport(el.clientHeight || 320);
    if (!following) return;
    el.scrollTop = el.scrollHeight;
    setScrollTop(el.scrollTop);
  }, [total, following]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => setViewport(el.clientHeight || 320));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const jump = () => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setFollowing(true);
  };

  const toggle = (seq: number) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(seq)) next.delete(seq);
      else next.add(seq);
      return next;
    });

  return (
    <div className="stream-log">
      <div className="response-body-toolbar">
        <div className="segmented" role="group" aria-label="Filter messages">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              className={`segment ${filter === f.id ? "segment-active" : ""}`}
              aria-pressed={filter === f.id}
              onClick={() => setFilter(f.id)}
            >
              {f.label}
            </button>
          ))}
        </div>
        <span className="header-spacer" />
        <div className="find-box">
          <span className="search-icon">
            <Search size={12} />
          </span>
          <input
            className="input find-input mono"
            placeholder="Filter the log"
            aria-label="Filter the log"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setQuery("");
            }}
          />
          {(query !== "" || filter !== "all") && (
            <span className="find-count">
              {shown.length}/{rows.length}
            </span>
          )}
        </div>
        <button
          className={`toggle ${following ? "toggle-on" : ""}`}
          aria-pressed={following}
          title="Follow the newest message"
          onClick={() => (following ? setFollowing(false) : jump())}
        >
          Follow
        </button>
        <button
          className="btn-ghost btn-icon"
          aria-label="Copy the whole log"
          title="Copy the whole log"
          onClick={() => {
            void navigator.clipboard.writeText(
              shown.map((row) => `${row.gutter} ${copyText(row)}`).join("\n"),
            );
            toast("success", `${shown.length} rows copied`);
          }}
        >
          <Copy size={13} />
        </button>
        <button
          className="btn-ghost btn-icon"
          aria-label="Clear the log"
          title="Clear the log"
          onClick={onClear}
        >
          <Trash size={13} />
        </button>
      </div>

      {overflow > 0 && (
        <div className="log-overflow" role="status">
          Showing the last {LOG_CAP.toLocaleString()} messages — {overflow.toLocaleString()}{" "}
          older {overflow === 1 ? "row is" : "rows are"} no longer kept.
        </div>
      )}

      <div className="log-scroll" ref={scrollRef} onScroll={onScroll} role="log">
        {shown.length === 0 ? (
          <div className="empty">
            <span className="empty-icon">
              <Inbox size={26} />
            </span>
            <p className="empty-line">
              {rows.length === 0
                ? "Nothing on this connection yet. Connect, and every frame lands here."
                : "No message matches this filter."}
            </p>
          </div>
        ) : (
          <div className="log-spacer" style={{ height: total }}>
            <div
              className="log-window"
              style={{ transform: `translateY(${offsets[start]}px)` }}
            >
              {window.map((row) => (
                <StreamLogRow
                  key={row.seq}
                  row={row}
                  expanded={expanded.has(row.seq)}
                  onToggle={() => toggle(row.seq)}
                />
              ))}
            </div>
          </div>
        )}
        {!following && shown.length > 0 && (
          <button className="log-jump" onClick={jump}>
            <ArrowDown size={12} />
            Jump to the newest
          </button>
        )}
      </div>
    </div>
  );
}
