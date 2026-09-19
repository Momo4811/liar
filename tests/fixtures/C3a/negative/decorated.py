# A decorator can change what calling this produces.
def wrap(f):
    return f


@wrap
def is_ready() -> str:
    return "yes"
