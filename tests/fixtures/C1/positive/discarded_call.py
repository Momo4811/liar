# A call that is nothing but a call. The coroutine is built and dropped.
async def save():
    pass


async def handle():
    save()  # expect: C1
