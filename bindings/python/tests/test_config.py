from outpunch import ClientConfig


def test_config_defaults():
    c = ClientConfig(server_url="ws://x", secret="s", service="svc")
    assert c.forward_to == "http://localhost:8080"
    assert c.reconnect_delay == 5.0
    assert c.request_timeout == 25.0


def test_config_custom_values():
    c = ClientConfig(
        server_url="wss://example.com/ws",
        secret="my-secret",
        service="my-app",
        forward_to="http://localhost:9000",
        reconnect_delay=10.0,
        request_timeout=30.0,
    )
    assert c.server_url == "wss://example.com/ws"
    assert c.secret == "my-secret"
    assert c.service == "my-app"
    assert c.forward_to == "http://localhost:9000"
    assert c.reconnect_delay == 10.0
    assert c.request_timeout == 30.0


def test_config_repr():
    c = ClientConfig(server_url="ws://x", secret="s", service="svc")
    r = repr(c)
    assert "ws://x" in r
    assert "svc" in r
    assert "http://localhost:8080" in r


def test_config_fields_mutable():
    c = ClientConfig(server_url="ws://x", secret="s", service="svc")
    c.forward_to = "http://localhost:3000"
    assert c.forward_to == "http://localhost:3000"
    c.reconnect_delay = 15.0
    assert c.reconnect_delay == 15.0
