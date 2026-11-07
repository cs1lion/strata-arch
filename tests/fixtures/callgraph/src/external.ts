import React from "react";
import * as fs from "node:fs";

export function useMemoValue(value: number) {
  return React.useMemo(() => value, [value]);
}

export function readIt(path: string) {
  return fs.readFileSync(path, "utf8");
}
