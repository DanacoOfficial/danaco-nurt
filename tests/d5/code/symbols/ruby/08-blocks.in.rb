square = ->(x) { x * x }

def with_block
  yield square.call(2)
end
