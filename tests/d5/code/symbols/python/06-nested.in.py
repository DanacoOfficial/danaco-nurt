def outer():
    def inner():
        return 1

    return inner


class A:
    class B:
        def method(self):
            return 2
