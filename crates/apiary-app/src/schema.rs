//! Diesel table definitions, mirroring `migrations/`.

diesel::table! {
    notes (id) {
        id -> BigInt,
        body -> Text,
        tenant_id -> Nullable<Text>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    offboardings (hive) {
        hive -> Text,
        workflow_id -> Text,
        phase -> Text,
        detail -> Text,
        backup -> Nullable<Text>,
        grace_until -> Nullable<Timestamp>,
        updated_at -> Timestamp,
    }
}
