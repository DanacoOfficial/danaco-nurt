function fixture() {
  return [1, 2];
}

describe("math", () => {
  it("adds", () => {
    expect(fixture().length).toBe(2);
  });
});
