import time


def inner():
    time.sleep(1)


def outer():
    inner()


async def handle():
    outer()  # expect: C2
