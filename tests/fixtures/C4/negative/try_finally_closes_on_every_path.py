def read_config(path):
    f = open(path)
    try:
        return parse(f.read())
    finally:
        f.close()
