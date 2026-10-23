"""Synchronous and asynchronous clients for the Moss HTTP API.

The generic ``get/post/put/delete`` methods work for every route in the frozen spec. The typed
convenience methods cover the most common operations and return models from ``models.py``.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional
from urllib.parse import quote

import httpx

from .models import Backup, Finding, Operation, Server

API_BASE_PATH = "/api/v1"
DEFAULT_TIMEOUT = 30.0


class MossError(Exception):
    """Raised for any non-2xx response. ``body`` is the parsed API error body when there is one."""

    def __init__(self, status: int, body: Any):
        self.status = status
        self.body = body
        err = body.get("error") if isinstance(body, dict) else None
        self.code: Optional[str] = err.get("code") if isinstance(err, dict) else None
        self.message: str = (err.get("message") if isinstance(err, dict) else None) or str(body)
        super().__init__(f"{status} {self.code or ''}: {self.message}".replace(" : ", ": "))


def _base(base_url: str) -> str:
    root = base_url.rstrip("/")
    return root if root.endswith(API_BASE_PATH) else root + API_BASE_PATH


def _seg(value: str) -> str:
    return quote(str(value), safe="")


def _parse(resp: httpx.Response) -> Any:
    try:
        body: Any = resp.json() if resp.content else None
    except ValueError:
        body = resp.text
    if resp.status_code >= 400:
        raise MossError(resp.status_code, body)
    return body


def _clean(params: Dict[str, Any]) -> Dict[str, Any]:
    out: Dict[str, Any] = {}
    for k, v in params.items():
        if v is None:
            continue
        out[k] = ("true" if v else "false") if isinstance(v, bool) else v
    return out


class _Typed:
    """Models-building helpers shared by both clients."""

    @staticmethod
    def servers(data: Any) -> List[Server]:
        return [Server.model_validate(s) for s in data]

    @staticmethod
    def server(data: Any) -> Server:
        return Server.model_validate(data)

    @staticmethod
    def backups(data: Any) -> List[Backup]:
        return [Backup.model_validate(b) for b in data]

    @staticmethod
    def operation(data: Any) -> Operation:
        return Operation.model_validate(data)

    @staticmethod
    def findings(data: Any) -> List[Finding]:
        return [Finding.model_validate(f) for f in data]


class MossClient:
    """Blocking client. ``token`` is a Moss API token (``moss_pat_...``)."""

    def __init__(
        self,
        base_url: str,
        token: str,
        *,
        timeout: float = DEFAULT_TIMEOUT,
        transport: Optional[httpx.BaseTransport] = None,
    ):
        self._http = httpx.Client(
            base_url=_base(base_url),
            headers={"Authorization": f"Bearer {token}"},
            timeout=timeout,
            transport=transport,
        )

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "MossClient":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def request(self, method: str, path: str, *, json: Any = None, **params: Any) -> Any:
        """Send a request. ``path`` is relative to ``/api/v1``; ``params`` become the query string."""
        resp = self._http.request(method, path, params=_clean(params), json=json)
        return _parse(resp)

    def get(self, path: str, **params: Any) -> Any:
        return self.request("GET", path, **params)

    def post(self, path: str, *, json: Any = None, **params: Any) -> Any:
        return self.request("POST", path, json=json, **params)

    def put(self, path: str, *, json: Any = None, **params: Any) -> Any:
        return self.request("PUT", path, json=json, **params)

    def delete(self, path: str, **params: Any) -> Any:
        return self.request("DELETE", path, **params)

    # Typed convenience methods

    def list_servers(self, include_staging: Optional[bool] = None) -> List[Server]:
        return _Typed.servers(self.get("/servers", include_staging=include_staging))

    def get_server(self, server_id: str) -> Server:
        return _Typed.server(self.get(f"/servers/{_seg(server_id)}"))

    def _action(self, server_id: str, action: str) -> Server:
        return _Typed.server(self.post(f"/servers/{_seg(server_id)}/actions/{action}"))

    def start_server(self, server_id: str) -> Server:
        return self._action(server_id, "start")

    def stop_server(self, server_id: str) -> Server:
        return self._action(server_id, "stop")

    def restart_server(self, server_id: str) -> Server:
        return self._action(server_id, "restart")

    def list_backups(self, server_id: str) -> List[Backup]:
        return _Typed.backups(self.get(f"/servers/{_seg(server_id)}/backups"))

    def create_backup(self, server_id: str, **body: Any) -> Operation:
        """Starts a backup; the API answers 202 with the running operation."""
        return _Typed.operation(self.post(f"/servers/{_seg(server_id)}/backups", json=body))

    def health(self) -> List[Finding]:
        return _Typed.findings(self.get("/health"))


class AsyncMossClient:
    """Async client with the same methods as :class:`MossClient`."""

    def __init__(
        self,
        base_url: str,
        token: str,
        *,
        timeout: float = DEFAULT_TIMEOUT,
        transport: Optional[httpx.AsyncBaseTransport] = None,
    ):
        self._http = httpx.AsyncClient(
            base_url=_base(base_url),
            headers={"Authorization": f"Bearer {token}"},
            timeout=timeout,
            transport=transport,
        )

    async def aclose(self) -> None:
        await self._http.aclose()

    async def __aenter__(self) -> "AsyncMossClient":
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.aclose()

    async def request(self, method: str, path: str, *, json: Any = None, **params: Any) -> Any:
        resp = await self._http.request(method, path, params=_clean(params), json=json)
        return _parse(resp)

    async def get(self, path: str, **params: Any) -> Any:
        return await self.request("GET", path, **params)

    async def post(self, path: str, *, json: Any = None, **params: Any) -> Any:
        return await self.request("POST", path, json=json, **params)

    async def put(self, path: str, *, json: Any = None, **params: Any) -> Any:
        return await self.request("PUT", path, json=json, **params)

    async def delete(self, path: str, **params: Any) -> Any:
        return await self.request("DELETE", path, **params)

    async def list_servers(self, include_staging: Optional[bool] = None) -> List[Server]:
        return _Typed.servers(await self.get("/servers", include_staging=include_staging))

    async def get_server(self, server_id: str) -> Server:
        return _Typed.server(await self.get(f"/servers/{_seg(server_id)}"))

    async def _action(self, server_id: str, action: str) -> Server:
        return _Typed.server(await self.post(f"/servers/{_seg(server_id)}/actions/{action}"))

    async def start_server(self, server_id: str) -> Server:
        return await self._action(server_id, "start")

    async def stop_server(self, server_id: str) -> Server:
        return await self._action(server_id, "stop")

    async def restart_server(self, server_id: str) -> Server:
        return await self._action(server_id, "restart")

    async def list_backups(self, server_id: str) -> List[Backup]:
        return _Typed.backups(await self.get(f"/servers/{_seg(server_id)}/backups"))

    async def create_backup(self, server_id: str, **body: Any) -> Operation:
        return _Typed.operation(await self.post(f"/servers/{_seg(server_id)}/backups", json=body))

    async def health(self) -> List[Finding]:
        return _Typed.findings(await self.get("/health"))
