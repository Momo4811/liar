# The callee is defined in another file. Resolution has to follow the import.
from helpers import save


async def handle():
    save()  # expect: C1
