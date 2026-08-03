export interface Scored<T> {
  item: T;
  score: number;
}

export function fuzzyScore(text: string, query: string): number {
  if (query === "") return 1;
  const haystack = text.toLowerCase();
  const needle = query.toLowerCase();
  const exact = haystack.indexOf(needle);
  if (exact === 0) return 1000;
  if (exact > 0) return 700 - exact;

  let score = 0;
  let index = 0;
  let streak = 0;
  for (const char of needle) {
    const found = haystack.indexOf(char, index);
    if (found === -1) return 0;
    streak = found === index ? streak + 1 : 0;
    score += 10 + streak * 4 - Math.min(found - index, 8);
    index = found + 1;
  }
  return Math.max(score, 1);
}

export function fuzzyFilter<T>(
  items: T[],
  query: string,
  key: (item: T) => string,
): T[] {
  if (query.trim() === "") return items;
  const scored: Scored<T>[] = [];
  for (const item of items) {
    const score = fuzzyScore(key(item), query.trim());
    if (score > 0) scored.push({ item, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((s) => s.item);
}
