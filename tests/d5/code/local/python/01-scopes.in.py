import os
from math import sqrt


def area(r):
    return 3 * r * r


def hypot(a, b):
    total = a * a + b * b
    return sqrt(total)


def main():
    r = area(2)
    print(hypot(r, 1), os.sep)
