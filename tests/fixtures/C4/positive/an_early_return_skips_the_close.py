def read_if_valid(path, flag):
    f = open(path)  # expect: C4
    if flag:
        return None
    f.close()
    return True
