"""Independent HTTPS wire oracle and held-open SSE fixture for C++ ownership."""
import json
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


def serve(raw):
    try:
        raw.settimeout(4)
        with context.wrap_socket(raw, server_side=True) as stream:
            request = b""
            while b"\r\n\r\n" not in request and len(request) <= 65536:
                data = stream.recv(4096)
                if not data:
                    return
                request += data
            head, body = request.split(b"\r\n\r\n", 1)
            line, *fields = head.decode("utf-8").split("\r\n")
            method, path, _ = line.split(" ")
            headers = dict((key.lower(), value.strip()) for key, value in
                           (field.split(":", 1) for field in fields))
            length = int(headers.get("content-length", "0"))
            if length > 65536:
                return
            while len(body) < length:
                data = stream.recv(min(4096, length - len(body)))
                if not data:
                    return
                body += data
            with lock, (root / "wire.jsonl").open("a") as log:
                log.write(json.dumps({"method": method, "path": path,
                                      "headers": headers, "body": list(body)}) + "\n")
            if path.startswith("/events/"):
                stream.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n"
                               b"Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                if not path.endswith("timeout"):
                    data = b"data: verified TLS\n\n"
                    stream.sendall(f"{len(data):x}\r\n".encode() + data + b"\r\n")
                try:
                    closed = stream.recv(1) == b""
                except (ConnectionResetError, ssl.SSLEOFError):
                    closed = True
                if closed:
                    (root / (path.rsplit("/", 1)[1] + "-closed")).write_text("closed")
            elif path in ("/extraForm", "/extraMultipart"):
                media = headers["content-type"].encode()
                stream.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: " + media +
                               b"\r\nConnection: close\r\nContent-Length: " +
                               str(len(body)).encode() + b"\r\n\r\n" + body)
            else:
                stream.sendall(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
    except (OSError, ssl.SSLError):
        raw.close()


while True:
    raw, _ = listener.accept()
    threading.Thread(target=serve, args=(raw,), daemon=True).start()
