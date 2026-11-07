import type { Pair } from "./types";

export function add(a: number, b: number): number {
  const pair: Pair = { a, b };
  return pair.a + pair.b;
}
