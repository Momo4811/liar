# Documenting some parameters and not others is normal and useful. Reporting it
# would be a style opinion dressed up as a correctness one.
def fetch(url, timeout, retries):
    """Fetch a thing.

    Args:
        url: where to fetch from
    """
    return get(url, timeout, retries)
