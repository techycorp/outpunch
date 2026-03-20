import pytest

from outpunch import ClientConfig, run_connection


@pytest.mark.asyncio
async def test_run_connection_fails_when_server_unreachable():
    config = ClientConfig(
        server_url="ws://127.0.0.1:1/ws",
        secret="secret",
        service="svc",
        forward_to="http://localhost:8080",
    )
    with pytest.raises(RuntimeError):
        await run_connection(config)
