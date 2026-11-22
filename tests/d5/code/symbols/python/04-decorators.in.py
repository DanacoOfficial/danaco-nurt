from dataclasses import dataclass


@dataclass
class Point:
    x: int
    y: int

    @property
    def norm(self):
        return self.x * self.x + self.y * self.y

    @staticmethod
    def origin():
        return Point(0, 0)
