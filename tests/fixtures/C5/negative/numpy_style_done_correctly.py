def fetch(url, timeout):
    """Fetch a thing.

    Parameters
    ----------
    url : str
        where to fetch from
    timeout : int
        how long to wait

    Returns
    -------
    str
        the body
    """
    return get(url, timeout)
