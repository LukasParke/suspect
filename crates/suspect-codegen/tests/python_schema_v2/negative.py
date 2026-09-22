from scoped_python_sdk import Client, models
def invalid(client: Client) -> None:
    client.write_conditional()
    models.Conditional(kind='other')
    models.Conditional(kind='s', count='not an int')
    models.PatternBag(fixed=1)
    client.write_tuple(body=['head', object()])
    models.NullFields(required=1)
    models.NullFields(required=None, items=[False])
