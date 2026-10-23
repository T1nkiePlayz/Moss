# moss-client (Python)

Python client for the [Moss](../../README.md) HTTP API. The models in `src/moss_client/models.py` are generated from
[docs/openapi.frozen.json](../../docs/openapi.frozen.json); the client in `src/moss_client/client.py` is hand-written.
Python 3.10+, `httpx` and `pydantic` v2.

## Install

```sh
pip install ./sdk/python        # from a checkout of the Moss repo
```

## Create an API token

In the Moss panel open **Settings** and create a token: give it a name and a scope (`read` allows `GET` only, `full`
allows everything the owning user can do) and an optional expiry. The token (`moss_pat_...`) is shown once. You can
also call `POST /tokens {name, scope, expires_in_days?}` from a signed-in session; a token cannot create tokens.
The client sends it as `Authorization: Bearer moss_pat_...`. All routes live under `/api/v1`, which the client adds.

## Examples

```python
from moss_client import MossClient, MossError

with MossClient("http://localhost:8080", "moss_pat_...") as moss:
    # 1. List servers (GET /servers)
    for s in moss.list_servers():
        print(s.id, s.name, s.state)

    # 2. Start a server (POST /servers/{id}/actions/start)
    server = moss.start_server("SERVER_ID")

    # 3. Create a backup (POST /servers/{id}/backups, answers 202 with an operation)
    op = moss.create_backup("SERVER_ID")
    print(op.id, op.state)

    # Any other route: generic calls return parsed JSON
    print(moss.get("/settings"))
    try:
        moss.post("/servers/SERVER_ID/actions/stop")
    except MossError as e:
        print(e.status, e.code, e.message)   # e.body is the full API error body
```

`AsyncMossClient` has the same methods, awaited (`async with AsyncMossClient(...) as moss`).

Typed helpers: `list_servers`, `get_server`, `start_server`, `stop_server`, `restart_server`, `list_backups`,
`create_backup`, `health`. The spec has no operation IDs, so everything else goes through `get/post/put/delete`.

## Develop

```sh
pip install -e './sdk/python[test]'
pytest sdk/python
```

Regenerate the models with `scripts/gen-sdk.sh` from the repo root. Never edit `models.py` by hand.
