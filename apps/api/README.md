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
- Google Workspace-aligned auth claim checks + role-based access control
- Postgres-backed persistence

## Run

```bash
cp .env.example .env
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/virivu
./scripts/apply_migrations.sh
cargo run
```

## Example Requests

Create organization:

```bash
curl -X POST http://localhost:8080/v1/organizations \
  -H "x-dev-user-email: admin@cingulum.org" \
  -H "Content-Type: application/json" \
  -d '{"name":"Acme Research Institute"}'
```

Token introspection (Google ID token):

```bash
curl -X POST http://localhost:8080/v1/auth/google/token-introspect \
  -H "Content-Type: application/json" \
  -d '{"id_token":"<google-id-token>"}'
```

Get health:

```bash
curl http://localhost:8080/health
```

## Notes

- `ALLOW_DEV_AUTH_BYPASS=true` allows local development auth via `x-dev-user-email`.
- `migrations/0002_dev_seed.sql` creates `admin@cingulum.org` with `platform_admin` role for local testing.
- For production, keep `ALLOW_DEV_AUTH_BYPASS=false` and enforce real Google token verification.
- Current Google token handling validates claims and domain; cryptographic signature verification is marked as a TODO before production.
