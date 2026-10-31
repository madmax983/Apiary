# ADR 0001 — One SQLite file per hive, in tenant mode, provisioned explicitly

Status: accepted, 2026-10-31

## Context

autumn-web 0.9's database fleet (its ADR 0019) offers two shapes: one file per
tenant (`mode = "tenant"`) or one file per routing slot shared by many tenants
(`mode = "slot"`). Apiary exists to make the fleet visible, and the visible
unit should be a tenant.

## Decision

- `mode = "tenant"`: a hive is a tenant is a file. The comb draws one cell per
  file.
- `create_on_demand = false`: a file exists only once somebody founds the hive
  (`POST /hives` → `DatabaseFleet::provision`). Walking into an unfounded hive
  is a `404`, not an empty file. Tenant ids come from requests; letting a
  request create a file is how a fleet root fills with typos.
- `HiveName` is stricter than the fleet's own key rules, so a validated name
  can never be why a fleet call fails (checked against the real fleet in
  `tests/name_vs_fleet.rs`).
- `max_open = 16`, deliberately small, so the storm makes the fleet close files
  in front of you.
- The repository is still `tenant_scoped` (and the `notes` table still carries
  `tenant_id`) even though the file already isolates the tenant. The same
  repository then works unchanged in slot mode.
- The tenant comes from the session (`[tenancy] source = "session"`), set when
  you found or walk into a hive. There is no login; see the README.

## Consequences

- The open count is not a hard cap. A config-built fleet protects any file
  used in the last two seconds (`DEFAULT_MIN_RESIDENCY`, not settable from
  `autumn.toml`), so a burst over many hives runs past `max_open` and the
  sweeper brings it back. Apiary shows this rather than hiding it, and the
  integration tests that need a hard cap build their fleet with
  `min_residency(Duration::ZERO)`.
- Each open file holds up to `pool_size` connections and three descriptors per
  connection. A storm can hold hundreds of files open at once, which can exceed
  a default macOS `ulimit -n` of 256. The test storm is sized under that.
