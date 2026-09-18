# Most calls live inside control flow, so this is the case that matters most.
async def save():
    pass


async def handle(flag):
    if flag:
        save()  # expect: C1
    else:
        save()  # expect: C1

    while flag:
        save()  # expect: C1

    try:
        save()  # expect: C1
    except ValueError:
        save()  # expect: C1
