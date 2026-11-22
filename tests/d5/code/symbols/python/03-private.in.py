def _helper():
    return 1


class _Internal:
    pass


def public():
    return _helper()
