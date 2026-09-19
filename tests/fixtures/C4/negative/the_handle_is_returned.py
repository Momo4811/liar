# Once it leaves, its lifetime is the caller's business.
def acquire(path):
    f = open(path)
    return f
