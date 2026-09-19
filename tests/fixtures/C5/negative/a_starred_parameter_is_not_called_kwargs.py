# Minimised from quart/app.py, reported wrongly.
#
# The docstring documents the names that go into `**options`. That is exactly
# what the catch-all rule is for, and it matched on the parameter being called
# `kwargs` rather than on it being starred.
def websocket(rule, **options):
    """Add a websocket.

    Arguments:
        rule: the path to route on
        endpoint: optional endpoint name
        defaults: variables to provide automatically
    """
    return register(rule, options)
