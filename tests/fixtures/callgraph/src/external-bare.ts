import clsx from "clsx";
import { CLIEngine } from "eslint";

export function classNames(value: string) {
  return clsx(value, "primary");
}

export function lint(files: string[]) {
  return new CLIEngine(files);
}
