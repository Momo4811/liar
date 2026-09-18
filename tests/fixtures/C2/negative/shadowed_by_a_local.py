# The local definition is what the bare name refers to, and it is not
# time.sleep.
from time import sleep


def sleep():
    pass


async def handle():
    sleep()
