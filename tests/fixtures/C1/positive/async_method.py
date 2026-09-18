# self.method() inside a class resolves through the class body.
class Service:
    async def save(self):
        pass

    async def handle(self):
        self.save()  # expect: C1
