def scan(paths):
    for path in paths:
        f = open(path)  # expect: C4
        use(f.read())
        f.close()
    return None
