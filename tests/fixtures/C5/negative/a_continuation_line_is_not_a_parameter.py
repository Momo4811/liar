# Minimised from sanic. The description continues on the next line and happens
# to contain a colon.
def serve(host, port):
    """Serve the app.

    Args:
        host: the host to bind
            Default: localhost
        port: the port to listen on
    """
    return run(host, port)
