# The inner definition is what the bare name refers to, and it is not async.
async def save():
    pass


def outer():
    def save():
        pass

    save()
