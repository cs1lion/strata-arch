import { alpha } from "./a";
import { Widget } from "./components";

export function caller() {
  alpha();
  return <Widget />;
}

export function unattributed(target: any) {
  target.unknownMethod();
}

export async function load() {
  return import("./a");
}

const dynamicTarget = "./a";

export function loadDynamic() {
  return import(dynamicTarget);
}
