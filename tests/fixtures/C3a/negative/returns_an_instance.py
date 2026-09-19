# A class can implement __bool__, so this name may be telling the truth.
class Maybe:
    pass


def is_ready() -> Maybe:
    return Maybe()
