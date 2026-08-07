export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/**
 * How far from the bottom still counts as "at the bottom". A chatty socket
 * appends while the user reads, so the test has to tolerate the pixel or two a
 * fresh row costs before the scroll event lands.
 */
export const FOLLOW_THRESHOLD = 24;

/**
 * Scrolling up pauses the follow; coming back to the bottom resumes it. Nothing
 * else may resume it — a log that yanks the reader back down is unusable.
 */
export function atBottom(m: ScrollMetrics, threshold = FOLLOW_THRESHOLD): boolean {
  return m.scrollHeight - m.scrollTop - m.clientHeight <= threshold;
}
