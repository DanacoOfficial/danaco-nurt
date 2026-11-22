function setup() {
  return render(<Button label="ok" />);
}

describe("Button", () => {
  it("renders", () => {
    setup();
  });
});
