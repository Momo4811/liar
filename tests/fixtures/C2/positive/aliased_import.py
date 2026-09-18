# The alias changes the spelling, not the module. Resolution sees through it.
import time as t


async def handle():
    t.sleep(1)  # expect: C2
