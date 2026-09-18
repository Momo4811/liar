# requests is not being analysed, so whether get is async is unknowable.
from requests import get


async def handle():
    get()
