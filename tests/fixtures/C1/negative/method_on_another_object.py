# Knowing whether service.save is async would need the type of `service`,
# which the engine cannot work out. Silence is the correct answer.
class Service:
    async def save(self):
        pass


async def handle(service):
    service.save()
