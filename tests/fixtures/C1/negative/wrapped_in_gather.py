import asyncio


async def save():
    pass


async def handle():
    await asyncio.gather(save(), save())
