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
- Electronic Data Use Agreement (DUA) creation + e-signature workflow

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

Create DUA (Cingulum-side):

```bash
curl -X POST http://localhost:8080/v1/legal/data-use-agreements \
  -H "x-dev-user-email: arcot@cingulum.org" \
  -H "Content-Type: application/json" \
  -d '{
    "organization_id":"<org-uuid>",
    "hospital_name":"Example Hospital",
    "hospital_contact_name":"Legal Contact",
    "hospital_contact_email":"legal@examplehospital.org",
    "agreement_version":"1.0",
    "effective_date":"2026-06-01",
    "expiration_date":"2027-06-01",
    "agreement_text":"<approved DUA text>"
  }'
```

Hospital e-sign (token-based):

```bash
curl -X POST http://localhost:8080/v1/legal/data-use-agreements/sign-hospital \
  -H "Content-Type: application/json" \
  -d '{
    "signing_token":"<hospital-signing-token>",
    "signer_name":"Hospital Signer",
    "signer_email":"signer@examplehospital.org",
    "signer_title":"Chief Medical Officer",
    "signer_organization":"Example Hospital",
    "signature_method":"typed",
    "signature_text":"/s/ Hospital Signer"
  }'
```

Cingulum e-sign:

```bash
curl -X POST http://localhost:8080/v1/legal/data-use-agreements/<agreement-id>/sign-cingulum \
  -H "x-dev-user-email: arcot@cingulum.org" \
  -H "Content-Type: application/json" \
  -d '{
    "signer_name":"Arcot",
    "signer_email":"arcot@cingulum.org",
    "signer_title":"Authorized Representative",
    "signature_method":"typed",
    "signature_text":"/s/ Arcot"
  }'
```

## Notes

- `ALLOW_DEV_AUTH_BYPASS=true` allows local development auth via `x-dev-user-email`.
- `migrations/0002_dev_seed.sql` creates `admin@cingulum.org` with `platform_admin` role for local testing.
- For production, keep `ALLOW_DEV_AUTH_BYPASS=false` and enforce real Google token verification.
- Current Google token handling validates claims and domain; cryptographic signature verification is marked as a TODO before production.
