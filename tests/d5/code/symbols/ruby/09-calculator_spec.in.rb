def build_calculator
  Calculator.new
end

describe Calculator do
  it "adds" do
    expect(build_calculator.add(1, 1)).to eq(2)
  end
end
