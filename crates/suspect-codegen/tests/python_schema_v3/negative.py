from resource_python_sdk import Client, codecs
def invalid(client: Client) -> None:
    client.write_override()
    client.write_override(body={'payload': 1.25})
    client.write_array(body=['head', object()])
    codecs.NumberCodec.encode(1.25)
    codecs.OverrideCodec.encode(['not an object'])
