# Minimised from celery/contrib/rdb.py, which really does leak here.
#
# On the `continue` path the loop goes round and binds a fresh socket while the
# previous one is still open. The handle does escape on the path that succeeds,
# so the escape rule alone would say nothing.
import socket


def find_free_port(host, first_port, limit):
    for i in range(limit):
        sock = socket.socket()  # expect: C4
        try:
            sock.bind((host, first_port + i))
        except OSError:
            continue
        else:
            return sock, first_port + i
    raise Exception("no port")
