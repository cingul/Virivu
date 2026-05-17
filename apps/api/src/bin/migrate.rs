use anyhow::Context;
use diesel::{pg::PgConnection, prelude::*, sql_query};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");
const MIGRATION_INIT_VARIANTS: &[&str] = &["2026-05-17-000001", "2026-05-17-000001_init"];
const MIGRATION_DUA_VARIANTS: &[&str] = &[
    "2026-05-17-000003",
    "2026-05-17-000003_data_use_agreements",
];

#[derive(diesel::deserialize::QueryableByName)]
struct ExistsRow {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    exists: bool,
}

fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;

    let mut connection = PgConnection::establish(&database_url)
        .with_context(|| "failed to connect to database using Diesel")?;

    bootstrap_legacy_schema_if_needed(&mut connection)?;
    let applied = connection
        .run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow::anyhow!("failed to run Diesel migrations: {e}"))?;

    if applied.is_empty() {
        println!("No pending Diesel migrations.");
    } else {
        println!("Applied {} migration(s):", applied.len());
        for migration in applied {
            println!(" - {}", migration);
        }
    }

    Ok(())
}

fn bootstrap_legacy_schema_if_needed(connection: &mut PgConnection) -> anyhow::Result<()> {
    sql_query(
        "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
            version VARCHAR(50) PRIMARY KEY,
            run_on TIMESTAMP NOT NULL DEFAULT NOW()
        )",
    )
    .execute(connection)
    .context("failed creating Diesel migration tracking table")?;

    let has_organizations =
        sql_query("SELECT to_regclass('public.organizations') IS NOT NULL AS exists")
            .get_result::<ExistsRow>(connection)
            .context("failed checking organizations table existence")?
            .exists;
    if has_organizations {
        for version in MIGRATION_INIT_VARIANTS {
            sql_query(format!(
                "INSERT INTO __diesel_schema_migrations(version, run_on)
                 VALUES ('{}', NOW())
                 ON CONFLICT (version) DO NOTHING",
                version
            ))
            .execute(connection)
            .context("failed baselining init migration")?;
        }
    }

    let has_dua_tables =
        sql_query("SELECT to_regclass('public.data_use_agreements') IS NOT NULL AS exists")
            .get_result::<ExistsRow>(connection)
            .context("failed checking DUA table existence")?
            .exists;
    if has_dua_tables {
        for version in MIGRATION_DUA_VARIANTS {
            sql_query(format!(
                "INSERT INTO __diesel_schema_migrations(version, run_on)
                 VALUES ('{}', NOW())
                 ON CONFLICT (version) DO NOTHING",
                version
            ))
            .execute(connection)
            .context("failed baselining DUA migration")?;
        }
    }

    Ok(())
}
