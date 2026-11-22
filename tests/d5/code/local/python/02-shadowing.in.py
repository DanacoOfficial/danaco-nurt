value = 1


def shadow(value):
    return value + 1


def closure():
    count = 0

    def bump():
        nonlocal count
        count += 1
        return count

    return bump
