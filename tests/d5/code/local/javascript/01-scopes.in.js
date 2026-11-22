import { readFile } from "fs";

function area(r) {
  return 3 * r * r;
}

function main() {
  const r = area(2);
  readFile("x", () => console.log(r));
}
