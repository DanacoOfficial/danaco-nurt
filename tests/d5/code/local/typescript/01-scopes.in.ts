import { readFile } from "fs";

function area(r: number): number {
  return 3 * r * r;
}

export function main(): void {
  const r = area(2);
  readFile("x", () => console.log(r));
}
