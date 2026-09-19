# A function taking **kwargs can legitimately document names that are not in
# its signature.
def configure(**kwargs):
    """Configure it.

    Args:
        host: where
        port: which
        timeout: how long
    """
    return dict(kwargs)
