# Virivu API (Rust)

Rust Axum API scaffold for Virivu Research Cloud.

## Capabilities currently scaffolded

- Health endpoint
- Organization / project / site creation endpoints
- Patient form invite endpoint
- Media upload pre-sign ticket endpoint
- Organization analytics summary endpoint
- Project progress report endpoint
- AI transcript-to-note placeholder endpoint

> Current persistence is in-memory for rapid iteration; SQL schema is included for Postgres migration planning.

## Run

```bash
cp .env.example .env
cargo run
```

## Example Requests

Create organization:

```bash
curl -X POST http://localhost:8080/v1/organizations \
  -H "Content-Type: application/json" \
  -d '{"name":"Acme Research Institute"}'
```

Get health:

```bash
curl http://localhost:8080/health
```
