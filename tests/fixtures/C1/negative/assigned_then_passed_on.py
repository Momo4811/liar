import asyncio


async def save():
    pass


async def handle():
    task = save()
    asyncio.create_task(task)
