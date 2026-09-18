# mystery is not being analysed, so whether fetch is async is unknowable.
# The library is deliberately not one C2 knows about either, so this fixture
# tests exactly one thing.
from mystery import fetch


async def handle():
    fetch()
