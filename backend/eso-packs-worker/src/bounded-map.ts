/** Store a value in insertion order while retaining at most `limit` entries. */
export function rememberBounded<K, V>(
  memo: Map<K, V>,
  key: K,
  value: V,
  limit: number,
): void {
  // Map preserves insertion order. Reinsert an existing key so frequently
  // refreshed entries do not remain the oldest forever.
  memo.delete(key);
  while (memo.size >= limit) {
    const oldest = memo.keys().next();
    if (oldest.done) break;
    memo.delete(oldest.value);
  }
  memo.set(key, value);
}
