import asyncio
import time


async def handle():
    await asyncio.to_thread(lambda: time.sleep(1))
