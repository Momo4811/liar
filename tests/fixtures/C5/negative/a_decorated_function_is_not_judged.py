# A decorator can change what the function is, and so what its docstring is
# describing.
def wrap(f):
    return f


@wrap
def save(record):
    """Save it.

    Returns:
        The new id.
    """
    store(record)
