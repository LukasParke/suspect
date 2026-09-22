"""Independent TLS fixture: a self-issued localhost certificate, no SDK codecs."""
import pathlib
import socket
import ssl
import sys
import threading

root = pathlib.Path(sys.argv[1])
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.minimum_version = ssl.TLSVersion.TLSv1_2
context.load_cert_chain(root / "server.crt", root / "server.key")
listener = socket.socket()
listener.bind(("127.0.0.1", 0))
listener.listen()
print(listener.getsockname()[1], flush=True)
lock = threading.Lock()
body = b'{"amount":1.25,"id":"tls","payload":{"kind":"standard","text":"trusted"}}'


def serve(raw):
    try:
        raw.settimeout(5)
        with context.wrap_socket(raw, server_side=True) as stream:
            request = b""
            while b"\r\n\r\n" not in request and len(request) < 65536:
                data = stream.recv(4096)
                if not data:
                    return
                request += data
            with lock, (root / "requests.txt").open("a") as log:
                log.write(request.split(b"\r\n")[0].decode("ascii") + "\n")
            stream.sendall(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: "
                + str(len(body)).encode("ascii") + b"\r\n\r\n" + body
            )
    except (OSError, ssl.SSLError):
        raw.close()


while True:
    raw, _ = listener.accept()
    threading.Thread(target=serve, args=(raw,), daemon=True).start()
