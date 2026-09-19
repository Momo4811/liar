# compute() returns something ordinary; garbage collection handles it.
def work(path):
    f = compute(path)
    data = parse(f.read())
    return data
