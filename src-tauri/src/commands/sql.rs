//! SQL -> Mongo translation command. The translator lives in the mongo
//! domain so the parsing logic is testable without a Tauri runtime.

use tauri::State;

use crate::error::AppResult;
use crate::mongo::sql_to_mongo::{translate, SqlTranslation};
use crate::state::AppState;

#[tauri::command]
pub async fn translate_sql(
    database: String,
    sql: String,
    _state: State<'_, AppState>,
) -> AppResult<SqlTranslation> {
    translate(&database, &sql)
}
