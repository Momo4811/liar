# The coroutine is handed to something else, which may well await it.
async def save():
    pass


def schedule(coro):
    return coro


async def handle():
    schedule(save())
