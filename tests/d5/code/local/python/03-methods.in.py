class Counter:
    def __init__(self, start):
        self.value = start

    def bump(self, step):
        self.value += step
        return self.helper(step)

    def helper(self, step):
        return step * 2
