# Minimised from tornado/test/testing_test.py, which the first corpus scan
# reported wrongly.
#
# gen_test turns a coroutine function into a synchronous one, so calling it
# without await is correct. A decorator this engine does not model can change
# what calling a function means, so nothing is said about decorated ones.
def gen_test(func):
    return func


@gen_test
async def run_it(self):
    self.finished = True


def test_native_coroutine(self):
    run_it(self)
