# Minimised from litestar/testing/client/subprocess_client.py.
#
# The helper does block, but it is wrapped in a decorator the engine does not
# model, and a decorator can just as easily hand the body to another thread.
# Propagating blocking through it would be reasoning from a signature that is
# no longer the real one.
import time
from contextlib import contextmanager


@contextmanager
def run_app():
    time.sleep(1)
    yield "url"


async def handle():
    with run_app() as url:
        return url
