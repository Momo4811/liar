import time

import anyio


async def handle():
    await anyio.to_thread.run_sync(lambda: time.sleep(1))
