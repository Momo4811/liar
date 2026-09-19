# The rest of tornado/test/util.py. The handle escapes through the return, but
# connect() can raise - and on that path nothing ever closes it.
#
# Reporting this is right. It is the canonical leak wearing a different hat:
# acquired, something raises, no cleanup on the way out.
import socket


def bind_unused_port(port):
    client_socket = socket.socket()  # expect: C4
    client_socket.connect(("127.0.0.1", port))
    return (client_socket.close, port)
