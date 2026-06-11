# QA Register Node Legacy Server

This directory contains the historical Node.js implementation of QA Register Server.

`registerserver/rustserver` is now the only primary maintained server implementation. This Node server is frozen as legacy and is kept only for historical behavior reference and emergency local fallback. Do not add new protocol fields, state-machine behavior, Web console expectations, qamcp behavior, or Unity client features here.

## Use Rust Server By Default

For development, verification, and release, run:

```powershell
cd registerserver/rustserver
cargo run
```

## Emergency Fallback

Stop the Rust server first because both implementations use port `3000` by default.

```powershell
cd registerserver/server
npm install
npm start
```

If the Web console static files are needed:

```powershell
cd registerserver/client
npm install
npm run build
```

## Maintenance Rule

- Rust server is the source of truth.
- Node legacy does not receive new features, regular bug fixes, protocol sync, or release validation.
- If Node behavior differs from Rust behavior, treat it as a legacy difference and document it instead of updating Node code.
- Only run `node --check src/index.js` when this legacy fallback itself must be inspected.

More context: `../../docs/NodeLegacy服务端说明.md`.
