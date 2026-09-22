#!/usr/bin/env python3
"""Loopback HTTP fixture for the generated Go CLI acceptance test.

Binds 127.0.0.1 on an ephemeral port, prints the port on the first stdout
line, then serves exactly the widget control-plane operations the fixture
contract declares. No upstream API is contacted and nothing is persisted.
Exact JSON number tokens are written as literal text so a large integer is
never routed through a float.
"""

import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

WIDGET = (
    '{{"id":{id},"amount":9007199254740993,"active":false,"note":null,"echo":{echo}}}'
)


def quoted(text):
    if text is None:
        return "null"
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        pass

    def reply(self, status, document):
        body = document.encode("utf-8") if document is not None else b""
        self.send_response(status)
        if body:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def echo(self, split, body=None):
        return (
            "{"
            + '"query":' + quoted(split.query)
            + ',"trace":' + quoted(self.headers.get("trace"))
            + ',"apiKey":' + quoted(self.headers.get("X-Api-Key"))
            + ',"body":' + quoted(body)
            + "}"
        )

    def do_GET(self):
        split = urlsplit(self.path)
        if split.path.startswith("/v1/widgets/"):
            identifier = split.path[len("/v1/widgets/"):]
            if identifier == "missing":
                self.reply(404, '{"message":"no such widget","code":404}')
                return
            self.reply(
                200,
                WIDGET.format(id=quoted(identifier), echo=self.echo(split)),
            )
            return
        self.reply(404, '{"message":"unrouted"}')

    def do_POST(self):
        split = urlsplit(self.path)
        length = int(self.headers.get("Content-Length") or "0")
        body = self.rfile.read(length).decode("utf-8") if length else ""
        if split.path == "/v1/widgets":
            if '"name":"deny"' in body:
                self.reply(422, '{"message":"rejected","code":422}')
                return
            self.reply(
                201,
                WIDGET.format(id=quoted("w-created"), echo=self.echo(split, body)),
            )
            return
        self.reply(404, '{"message":"unrouted"}')

    def do_DELETE(self):
        split = urlsplit(self.path)
        if split.path == "/v1/widgets":
            if "scope=locked" in (split.query or ""):
                self.reply(409, '{"message":"scope is locked","code":409}')
                return
            self.reply(204, None)
            return
        self.reply(404, '{"message":"unrouted"}')


def main():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    sys.stdout.write("%d\n" % server.server_address[1])
    sys.stdout.flush()
    server.serve_forever()


if __name__ == "__main__":
    main()
