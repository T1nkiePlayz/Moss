import json

import httpx
import pytest

from moss_client import AsyncMossClient, MossClient, MossError
from moss_client.models import Server


def make(handler):
    return MossClient("http://moss.test/", "moss_pat_abc", transport=httpx.MockTransport(handler))


def test_list_servers_sends_bearer_and_parses_models():
    seen = {}

    def handler(req: httpx.Request) -> httpx.Response:
        seen["url"] = str(req.url)
        seen["auth"] = req.headers["authorization"]
        return httpx.Response(200, json=[{"id": "s1", "name": "Survival", "state": "running"}])

    servers = make(handler).list_servers(include_staging=True)
    assert seen["url"] == "http://moss.test/api/v1/servers?include_staging=true"
    assert seen["auth"] == "Bearer moss_pat_abc"
    assert isinstance(servers[0], Server) and servers[0].name == "Survival"


def test_start_server_posts_action():
    seen = {}

    def handler(req: httpx.Request) -> httpx.Response:
        seen["m"], seen["p"] = req.method, req.url.raw_path.decode()
        return httpx.Response(200, json={"id": "a b", "state": "starting"})

    assert make(handler).start_server("a b").state == "starting"
    assert seen == {"m": "POST", "p": "/api/v1/servers/a%20b/actions/start"}


def test_create_backup_returns_operation_on_202():
    def handler(req: httpx.Request) -> httpx.Response:
        assert json.loads(req.content) == {}
        return httpx.Response(202, json={"id": "op1", "state": "queued"})

    op = make(handler).create_backup("s1")
    assert op.id == "op1"


def test_list_backups_and_health():
    def handler(req: httpx.Request) -> httpx.Response:
        if req.url.path.endswith("/backups"):
            return httpx.Response(200, json=[{"id": "b1"}])
        return httpx.Response(200, json=[{"id": "f1"}])

    c = make(handler)
    assert c.list_backups("s1")[0].id == "b1"
    assert c.health()[0].id == "f1"


def test_error_body_raises_moss_error():
    def handler(req: httpx.Request) -> httpx.Response:
        return httpx.Response(403, json={"error": {"code": "auth.token_scope", "message": "read-only token"}})

    with pytest.raises(MossError) as ei:
        make(handler).post("/servers/s1/actions/stop")
    assert ei.value.status == 403
    assert ei.value.code == "auth.token_scope"
    assert ei.value.body["error"]["message"] == "read-only token"


def test_generic_put_and_delete():
    calls = []

    def handler(req: httpx.Request) -> httpx.Response:
        calls.append((req.method, req.url.path, req.content))
        return httpx.Response(200, json={"ok": True})

    c = make(handler)
    assert c.put("/settings", json={"backup_keep": 5}) == {"ok": True}
    c.delete("/tokens/t1")
    assert calls[0][:2] == ("PUT", "/api/v1/settings") and json.loads(calls[0][2]) == {"backup_keep": 5}
    assert calls[1][:2] == ("DELETE", "/api/v1/tokens/t1")


async def test_async_client():
    def handler(req: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json=[{"id": "s1"}])

    async with AsyncMossClient("http://moss.test", "t", transport=httpx.MockTransport(handler)) as c:
        assert (await c.list_servers())[0].id == "s1"
