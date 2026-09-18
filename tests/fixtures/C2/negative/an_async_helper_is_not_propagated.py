# The blocking call inside the async helper is reported there, on its own line.
# Reporting it again at every call site would report one bug many times.
import time


async def helper():
    time.sleep(1)  # expect: C2


async def handle():
    await helper()
