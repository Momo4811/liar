# A plain def inside an async def is not itself async, so a blocking call in it
# blocks nothing that was not already blocked.
import time


async def outer():
    def inner():
        time.sleep(1)

    return inner
