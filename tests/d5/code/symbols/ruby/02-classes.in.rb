class Animal
  def initialize(name)
    @name = name
  end

  def speak; end
end

class Dog < Animal
  def speak
    "woof"
  end
end
