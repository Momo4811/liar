def collect(paths):
    handles = []
    for path in paths:
        f = open(path)
        handles.append(f)
    return handles
