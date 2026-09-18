import requests


async def fetch_all(urls):
    for url in urls:
        requests.get(url)  # expect: C2
