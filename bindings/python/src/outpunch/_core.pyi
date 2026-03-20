from typing import Any, Coroutine

class ClientConfig:
    server_url: str
    secret: str
    service: str
    forward_to: str
    reconnect_delay: float
    request_timeout: float

    def __init__(
        self,
        server_url: str,
        secret: str,
        service: str,
        forward_to: str = "http://localhost:8080",
        reconnect_delay: float = 5.0,
        request_timeout: float = 25.0,
    ) -> None: ...
    def __repr__(self) -> str: ...

def run(config: ClientConfig) -> None: ...
def run_connection(config: ClientConfig) -> Coroutine[Any, Any, None]: ...
