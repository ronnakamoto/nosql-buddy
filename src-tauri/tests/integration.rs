//! Integration tests for the NoSQLBuddy Rust backend.
//!
//! Pure logic (redaction, BSON<->JSON, SQL translation, secret store) is
//! tested without a Tauri runtime. The Mongo driver code paths are covered
//! by the in-memory tests + a separate integration test suite that
//! requires a live MongoDB (skipped when NOSQLBUDDY_TEST_URI is unset).

use app_lib::mongo::credentials::InMemorySecretStore;
use app_lib::mongo::credentials::SecretStore;
use app_lib::mongo::redaction::Redactor;
use app_lib::mongo::sql_to_mongo::translate;
use app_lib::mongo::types::mask_uri;

#[test]
fn redacts_credentials_in_error_messages() {
    let r = Redactor::new();
    let out = r.redact("connection error: mongodb://alice:hunter2@127.0.0.1:27017/?x=1");
    assert!(!out.contains("hunter2"), "leaked secret: {out}");
    assert!(!out.contains("alice"), "leaked user: {out}");
    assert!(out.contains("***:***@"), "expected mask: {out}");
}

#[test]
fn in_memory_secret_store_round_trip() {
    let store = InMemorySecretStore::new();
    store.put("p1", "secret").expect("put");
    assert_eq!(store.get("p1").expect("get"), Some("secret".to_string()));
    store.delete("p1").expect("delete");
    assert_eq!(store.get("p1").expect("get"), None);
}

#[test]
fn mask_uri_removes_userinfo() {
    let masked = mask_uri("mongodb://bob:pw@127.0.0.1:27017/db?retryWrites=true");
    assert_eq!(
        masked,
        "mongodb://***:***@127.0.0.1:27017/db?retryWrites=true"
    );
}

#[test]
fn sql_translate_select_with_where_and_limit() {
    let t = translate(
        "shop",
        "SELECT name FROM products WHERE price > 100 ORDER BY price DESC LIMIT 25",
    )
    .expect("translate");
    let stages = t.pipeline.as_array().expect("array");
    assert!(stages.iter().any(|s| s.get("$match").is_some()));
    assert!(stages.iter().any(|s| s.get("$sort").is_some()));
    assert!(stages.iter().any(|s| s.get("$limit").is_some()));
}

#[test]
fn sql_translate_join_to_lookup() {
    let t = translate(
        "shop",
        "SELECT u.name, o.total FROM users u JOIN orders o ON u._id = o.userId",
    )
    .expect("translate");
    let stages = t.pipeline.as_array().expect("array");
    let lookup = stages
        .iter()
        .find(|s| s.get("$lookup").is_some())
        .expect("lookup stage");
    assert_eq!(lookup["$lookup"]["from"], "orders");
    assert_eq!(lookup["$lookup"]["localField"], "_id");
    assert_eq!(lookup["$lookup"]["foreignField"], "userId");
}

#[test]
fn sql_translate_group_by_with_count() {
    let t = translate(
        "shop",
        "SELECT category, COUNT(*) AS c FROM products GROUP BY category",
    )
    .expect("translate");
    let stages = t.pipeline.as_array().expect("array");
    let group = stages
        .iter()
        .find(|s| s.get("$group").is_some())
        .expect("group stage");
    assert!(group["$group"]["_id"].is_object());
    assert_eq!(group["$group"]["c"]["$sum"], 1);
}

#[test]
fn sql_translate_rejects_non_select() {
    let res = translate("shop", "DROP TABLE products");
    assert!(res.is_err());
}

#[test]
fn sql_translate_update_basic() {
    let t = translate("shop", "UPDATE products SET price = 10 WHERE status = \"active\"").expect("translate");
    assert_eq!(t.collection, "products");
    assert_eq!(t.pipeline.as_array().unwrap().len(), 0);
}

#[test]
fn sql_translate_insert_basic() {
    let t = translate("shop", "INSERT INTO products VALUES {\"name\":\"A\"}").expect("translate");
    assert_eq!(t.collection, "products");
}

#[test]
fn sql_translate_delete_basic() {
    let t = translate("shop", "DELETE FROM products WHERE status = \"archived\"").expect("translate");
    assert_eq!(t.collection, "products");
}

#[test]
fn bson_round_trip_preserves_object_id() {
    use app_lib::mongo::bson_json::doc_to_extjson;
    use bson::doc;
    let oid = bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").expect("oid");
    let doc = doc! { "_id": oid, "name": "n" };
    let json = doc_to_extjson(&doc).expect("encode");
    let id = json.get("_id").expect("id present");
    assert!(id.is_object());
    assert_eq!(id["$oid"], "507f1f77bcf86cd799439011");
}

#[test]
fn bson_display_json_strips_extjson_for_oid() {
    use app_lib::mongo::bson_json::doc_to_display_json;
    use bson::doc;
    let oid = bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").expect("oid");
    let doc = doc! { "_id": oid, "name": "n" };
    let json = doc_to_display_json(&doc).expect("encode");
    assert!(json.get("_id").is_some());
    assert!(json["_id"].get("$oid").is_none() || json["_id"].get("_idDisplay").is_some());
}
