class Config
  def self.load(path)
    new
  end

  class << self
    def reset; end
  end
end
