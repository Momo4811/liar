# The language already solves this. A `with` target is not tracked at all.
def read_config(path):
    with open(path) as f:
        return parse(f.read())
