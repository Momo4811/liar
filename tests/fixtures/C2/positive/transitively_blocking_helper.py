# The helper is sync and blocks, so calling it from async code blocks too.
# Reported at the call site, because the helper itself is not async and so is
# never reported directly.
import time


def wait_a_bit():
    time.sleep(1)


async def handle():
    wait_a_bit()  # expect: C2
