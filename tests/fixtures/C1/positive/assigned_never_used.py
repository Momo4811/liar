# Bound to a name that is never mentioned again, so nothing can await it.
async def save():
    pass


async def handle():
    result = save()  # expect: C1
