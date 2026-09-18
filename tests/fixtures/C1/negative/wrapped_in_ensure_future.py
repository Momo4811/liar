import asyncio


async def save():
    pass


async def handle():
    asyncio.ensure_future(save())
