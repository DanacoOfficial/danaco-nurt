from typing import Protocol


class Reader(Protocol):
    def read(self) -> bytes: ...


def typed(value: int, *, scale: float = 1.0) -> str:
    return str(value * scale)
