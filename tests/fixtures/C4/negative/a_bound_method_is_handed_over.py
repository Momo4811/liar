# Minimised from tornado/test/util.py.
#
# Returning `client_socket.close` hands the caller a bound method and with it
# the responsibility for closing. A called attribute is a use; an uncalled one
# is an escape, and an escaped handle is somebody else's to close.
#
# Nothing between the acquisition and the return can raise, so there is no path
# on which the handle is lost.
import socket


def make_closer():
    client_socket = socket.socket()
    return (client_socket.close, 0)
