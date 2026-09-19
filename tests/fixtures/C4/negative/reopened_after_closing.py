# Opening a fresh handle each time round is fine when the previous one was
# closed first.
def scan(paths):
    for path in paths:
        f = open(path)
        f.close()
    return None
