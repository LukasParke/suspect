from openrouter_sdk import Client, AsyncClient, UNSET
Client(auth='not a mapping')  # rejected
AsyncClient(auth={'apiKey': UNSET})  # rejected
Client(timeout='not a timeout')  # rejected
