def read(path, flag):
    f = open(path)  # expect: C4
    if flag:
        f.close()
    return None
