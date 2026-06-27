//! MongoDB domain — connection registry, profile storage, driver client pool,
//! BSON<->JSON helpers, redaction, and SQL->Mongo translation. These modules
//! are intentionally separate from the Tauri command layer so the domain
//! logic can be unit-tested without a Tauri runtime.

pub mod bson_json;
pub mod client_registry;
pub mod credentials;
pub mod import_export;
pub mod job_store;
pub mod profiles;
pub mod query_code;
pub mod redaction;
pub mod schema;
pub mod shell;
pub mod shell_autocomplete;
pub mod sql_to_mongo;
pub mod types;
pub mod vqb;
