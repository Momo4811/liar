# A generator has no `return` with a value, so the engine infers None - but
# `yield` is not modelled, so it cannot tell a generator from a procedure. With
# no return statement at all there are too many benign explanations.
def rows(source):
    """Read the rows.

    Yields:
        Each row in turn.
    """
    for line in source:
        yield line.strip()
