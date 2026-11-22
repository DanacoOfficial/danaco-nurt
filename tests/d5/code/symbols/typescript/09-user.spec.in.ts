import { greet } from "./user";

function fixture(): string {
  return "ada";
}

describe("user", () => {
  it("greets", () => {
    expect(greet(fixture())).toBe("hello ada");
  });
});
