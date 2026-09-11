from credential_python_sdk import Client, AsyncClient, UNSET
Client(auth='token is not a mapping')  # rejected
AsyncClient(auth=17)  # rejected
Client(auth={'apiKey': UNSET})  # rejected
Client(timeout='not a timeout')  # rejected
