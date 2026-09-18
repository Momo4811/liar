import asyncio
import time


async def handle():
    loop = asyncio.get_running_loop()
    await loop.run_in_executor(None, lambda: time.sleep(1))
