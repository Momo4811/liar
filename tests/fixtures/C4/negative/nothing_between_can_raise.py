# open can raise, but at that point nothing is bound yet.
def touch(path):
    f = open(path)
    f.close()
    return None
