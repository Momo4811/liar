# Matching on the name alone would fire here. Matching on the resolved module
# path does not, because there is no module to resolve.
class Timer:
    def sleep(self, n):
        pass


async def handle(timer):
    timer.sleep(1)
