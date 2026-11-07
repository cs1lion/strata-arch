import { helper } from "./util";

export function alpha() {
  return helper();
}

export const beta = () => alpha();

export class Service {
  run() {
    return alpha();
  }
}
