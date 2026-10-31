async def save_user(user):
    pass


async def handle(request):
    save_user(request.user)


def sync_handle(request):
    save_user(request.user)
