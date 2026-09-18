# Calling an async function from a sync one and discarding it is still a bug:
# the body never runs.
async def save():
    pass


def handle():
    save()  # expect: C1
