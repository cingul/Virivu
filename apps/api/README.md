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
- DUA admin UI, hospital signing page, and PDF agreement export
- Outbound email queue for hospital signing links

## Run

```bash
cp .env.example .env
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/virivu
./scripts/apply_migrations.sh
cargo run
```

`./scripts/apply_migrations.sh` runs Diesel migrations and is safe to re-run.
It uses Diesel's migration tracking table (`__diesel_schema_migrations`) and includes a compatibility bootstrap for legacy local databases.

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

Queue hospital signing email:

```bash
curl -X POST http://localhost:8080/v1/legal/data-use-agreements/<agreement-id>/send-hospital-sign-link \
  -H "x-dev-user-email: arcot@cingulum.org"
```

Download DUA PDF:

```bash
curl -L http://localhost:8080/v1/legal/data-use-agreements/<agreement-id>/export.pdf \
  -H "x-dev-user-email: arcot@cingulum.org" \
  -o dua.pdf
```

Open DUA admin UI:

```bash
xdg-open http://localhost:8080/ui/dua
```

The DUA UI now includes an in-browser **Step 1** to create organizations, so you no longer need terminal commands just to get an organization UUID.

## Notes

- `ALLOW_DEV_AUTH_BYPASS=true` allows local development auth via `x-dev-user-email`.
- Diesel migration `2026-05-17-000002_dev_seed` seeds `admin@cingulum.org` and `arcot@cingulum.org` as `platform_admin` for local testing.
- Diesel migration `2026-05-17-000003_data_use_agreements` includes DUA tables and `outbound_emails` queue table.
- For production, keep `ALLOW_DEV_AUTH_BYPASS=false` and enforce real Google token verification.
- Current Google token handling validates claims and domain; cryptographic signature verification is marked as a TODO before production.
