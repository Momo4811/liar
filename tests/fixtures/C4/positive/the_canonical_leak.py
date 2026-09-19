# The example the whole check exists for. Fine on the happy path; leaks a
# handle every time parse raises.
def read_config(path):
    f = open(path)  # expect: C4
    data = parse(f.read())
    f.close()
    return data
