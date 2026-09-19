# It has a return, and the return gives nothing.
def save(record):  # expect: C5
    """Save the record.

    Returns:
        The saved record's id.
    """
    store(record)
    return
