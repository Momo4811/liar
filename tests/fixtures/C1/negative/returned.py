# A sync function returning a coroutine is a legitimate pattern.
async def save():
    pass


def handle():
    return save()
