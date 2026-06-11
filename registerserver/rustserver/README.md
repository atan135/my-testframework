# QA Register Rust Server

Rust implementation of QA Register Server. This is the only primary maintained server implementation.

The legacy Node.js server under `../server` is frozen and kept only as an emergency local fallback and historical behavior reference. New protocol fields, state-machine changes, bug fixes, Web console behavior, qamcp behavior, and production behavior must be implemented here, not in Node legacy.

It provides the public surface expected by the Web console, Unity client, and qamcp:

- `GET /api/health`
- `GET /api/unity-clients`
- `GET /api/results`
- `POST /api/artifacts?clientId=...&kind=screenshot&fileName=...`
- `GET /api/artifacts/:artifactId`
- `GET /api/artifacts/:artifactId/download`
- `POST /api/unity-clients/:clientId/execute`
- `WS /ws?role=unity`
- `WS /ws?role=web`
- static hosting for the configured web console `dist` directory

## Run

```powershell
cd registerserver/rustserver
cargo run
```

The default port is `3000`. Supported environment variables:

```powershell
$env:PORT=3001
$env:QA_LISTEN_HOST="0.0.0.0"
$env:QA_ACCESS_SCOPE="private"
$env:QA_CLIENT_DIST_DIR="C:\deploy\qa-register\client\dist"
$env:QA_ARTIFACT_DIR="C:\deploy\qa-register\artifacts"
$env:QA_ARTIFACT_MAX_BYTES=20971520
$env:EXECUTION_TIMEOUT_MS=20000
$env:UNITY_HEARTBEAT_STALE_MS=45000
$env:WS_HEARTBEAT_INTERVAL_MS=15000
$env:QA_WS_OUTBOUND_QUEUE_SIZE=1024
$env:QA_EXECUTION_ARCHIVE_MYSQL_ENABLED="false"
$env:QA_EXECUTION_ARCHIVE_MYSQL_URL=""
```

`QA_ACCESS_SCOPE` defaults to `private`, which accepts loopback, RFC1918 private networks, link-local addresses, and IPv6 local networks. Set `QA_ACCESS_SCOPE=unrestricted` only when an external firewall, gateway, or reverse proxy already protects the service.

The server loads `.env` from the current working directory first. If none is found there, it also tries `.env` next to the executable, which is the recommended Windows deployment layout.

Artifact uploads default to an `artifacts` directory under the server process current working directory. Override with `QA_ARTIFACT_DIR` if the upload store should live elsewhere. `QA_ARTIFACT_MAX_BYTES` defaults to `20971520` bytes.

Upload a screenshot artifact with raw bytes:

```powershell
curl.exe -X POST "http://localhost:3000/api/artifacts?clientId=<clientId>&kind=screenshot&fileName=screen.png" `
  -H "Content-Type: image/png" `
  --data-binary "@screen.png"
```

The response includes `artifactId`, `sha256`, and `downloadUrl`. Download with `GET /api/artifacts/<artifactId>/download`.

MariaDB/MySQL execution history archive is disabled by default. When `QA_EXECUTION_ARCHIVE_MYSQL_ENABLED=true`, the server connects to `QA_EXECUTION_ARCHIVE_MYSQL_URL` and writes execution records through a background queue; archive failures are logged and do not change the existing in-memory `/api/results` behavior.

Build the web console first if you want the Rust server to host the production UI:

```powershell
cd registerserver/client
npm run build
cd ../rustserver
cargo run --release
```

## Compatibility Notes

The WebSocket handler accepts both text frames and binary frames containing JSON. Both frame types share the same payload handling for Unity and Web roles, including `register`, `heartbeat`, `qa_result`, `refresh`, `execute`, `execute_sequence`, `stop_sequence`, and `stop_execution`.

Unity `register` messages may include `ipAddress` and `ipAddresses`. The Rust server exposes them to Web/qamcp snapshots and also adds `remoteAddress`, the peer address observed by the server.

Unity `register` and `heartbeat` messages may include `busy`, `currentRequestId`, and `currentMethodName`. The Rust server stores these values, exposes them to Web/qamcp snapshots as `clientBusy`, `currentRequestId`, and `currentMethodName`, and broadcasts `unity_state_changed` when they change.
