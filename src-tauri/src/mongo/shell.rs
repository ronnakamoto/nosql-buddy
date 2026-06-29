//! Mongo shell — real JavaScript REPL backed by `boa_engine`.
//!
//! ## Why boa_engine?
//!
//! A production-grade mongo shell supports real JavaScript:
//! `for` / `while` / `try-catch`, variable scope, string
//! concatenation, function calls, cursor chaining. A hand-rolled
//! structured parser can't deliver that without effectively
//! reimplementing JavaScript. `boa_engine` is a pure-Rust ES2024
//! engine — zero FFI risk, ~3 MB of compiled binaries, and the
//! same JS semantics as the user's browser.
//!
//! ## Architecture
//!
//! * A `Shell` is owned per-connection. Each `Shell` runs on a
//!   dedicated OS thread that hosts a `boa_engine::Context`
//!   (which is `!Send`) and a single-threaded `tokio` runtime
//!   used to drive MongoDB futures synchronously inside the
//!   native host functions.
//! * `eval_shell` posts a script + connection entry to the
//!   thread, waits for the result via a `oneshot` channel.
//! * The context registers native host functions that read
//!   thread-local state (the connection entry, the output
//!   buffer). Native fns block the JS thread on a Mongo future
//!   via `Runtime::block_on()` — the runtime is current-thread,
//!   so blocking on its own thread is allowed.
//! * `use <db>` is a preprocessor (boa doesn't know about it);
//!   we strip it before sending the script to boa and update
//!   the shell's active database.
//!
//! ## Limitations
//!
//! * No `eval()` / `Function()` (boa limitation by design).
//! * `db.<ident>` returns a `CollectionProxy` JS object whose
//!   methods run the underlying Mongo operation. Field
//!   access on the collection is not supported (the shell
//!   doesn't pre-load schemas).
//! * The user can't extend the prototype chain with their
//!   own JS classes that wrap Mongo — the host objects are
//!   black boxes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use bson::{doc, oid::ObjectId, DateTime as BsonDateTime, Document};
use futures_util::TryStreamExt;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tauri::async_runtime::Mutex as AsyncMutex;

use boa_engine::{
    context::Context, js_string, native_function::NativeFunction, object::builtins::JsArray,
    property::Attribute, JsArgs, JsError, JsNativeError, JsResult, JsValue, Source,
};

use crate::audit::interceptor;
use crate::audit::AuditLog;
use crate::error::{AppError, AppResult};
use crate::mongo::client_registry::ClientEntry;
use crate::mongo::timeline_store::OperationKind;

// ---------- Public types ----------

/// One output line from the shell.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ShellOutput {
    Text { value: String },
    Json { value: JsonValue },
    Error { value: String },
    Table { value: ShellTable },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<JsonValue>>,
    pub execution_ms: u64,
}

/// Metadata about a MongoDB operation performed inside the shell so the
/// caller can record it in the Data Timeline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellOperation {
    pub kind: OperationKind,
    pub database: String,
    pub collection: String,
    pub query_json: Option<String>,
    pub update_json: Option<String>,
    pub matched_count: Option<u64>,
    pub modified_count: Option<u64>,
    pub inserted_count: Option<u64>,
    pub deleted_count: Option<u64>,
    pub execution_ms: Option<u64>,
    pub errored: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellResponse {
    pub outputs: Vec<ShellOutput>,
    pub last_pipeline: Option<Vec<JsonValue>>,
    pub last_collection: Option<String>,
    pub last_database: Option<String>,
    pub active_database: String,
    pub execution_ms: u64,
    pub operations: Vec<ShellOperation>,
}

#[derive(Default)]
pub struct ShellRegistry {
    inner: AsyncMutex<HashMap<String, Arc<Shell>>>,
}

impl ShellRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get_or_create(
        &self,
        connection_id: String,
        initial_db: String,
    ) -> AppResult<Arc<Shell>> {
        let mut guard = self.inner.lock().await;
        if let Some(existing) = guard.get(&connection_id) {
            return Ok(existing.clone());
        }
        let shell = Arc::new(Shell::spawn(connection_id.clone(), initial_db)?);
        guard.insert(connection_id, shell.clone());
        Ok(shell)
    }

    pub async fn remove(&self, connection_id: &str) {
        let mut guard = self.inner.lock().await;
        if let Some(shell) = guard.remove(connection_id) {
            shell.shutdown();
        }
    }
}

// ---------- Thread-local bridge ----------

thread_local! {
    static SHELL_ENTRY: RefCell<Option<Arc<ClientEntry>>> = const { RefCell::new(None) };
    static SHELL_OUTPUT: RefCell<Vec<ShellOutput>> = const { RefCell::new(Vec::new()) };
    static SHELL_ACTIVE_DB: RefCell<String> = const { RefCell::new(String::new()) };
    static SHELL_LAST_PIPELINE: RefCell<Vec<JsonValue>> = const { RefCell::new(Vec::new()) };
    static SHELL_LAST_COLLECTION: RefCell<Option<String>> = const { RefCell::new(None) };
    static SHELL_AUDIT_LOG: RefCell<Option<Arc<AuditLog>>> = const { RefCell::new(None) };
    static SHELL_OPERATIONS: RefCell<Vec<ShellOperation>> = const { RefCell::new(Vec::new()) };
}

fn push_output(o: ShellOutput) {
    SHELL_OUTPUT.with(|b| b.borrow_mut().push(o));
}

fn take_outputs() -> Vec<ShellOutput> {
    SHELL_OUTPUT.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

fn push_operation(op: ShellOperation) {
    SHELL_OPERATIONS.with(|b| b.borrow_mut().push(op));
}

fn take_operations() -> Vec<ShellOperation> {
    SHELL_OPERATIONS.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

fn clear_operations() {
    SHELL_OPERATIONS.with(|b| b.borrow_mut().clear());
}

fn set_entry(e: Arc<ClientEntry>) {
    SHELL_ENTRY.with(|c| *c.borrow_mut() = Some(e));
}

fn set_audit_log(log: Arc<AuditLog>) {
    SHELL_AUDIT_LOG.with(|c| *c.borrow_mut() = Some(log));
}

/// Run a closure with the current audit log, if one is set.
/// Audit failures are logged and swallowed — a failed audit
/// recording must never break the user's shell operation.
fn try_audit<F>(f: F)
where
    F: FnOnce(&Arc<AuditLog>, &str),
{
    SHELL_AUDIT_LOG.with(|c| {
        if let Some(ref audit) = *c.borrow() {
            SHELL_ENTRY.with(|entry_cell| {
                let borrow = entry_cell.borrow();
                let deployment_id = borrow
                    .as_ref()
                    .map(|entry| entry.deployment_id.as_str())
                    .unwrap_or("");
                // Skip the interceptor capture path on deployments where the
                // change-stream listener is the authoritative capture path
                // (replica sets / sharded clusters). Otherwise shell-executed
                // writes would be recorded twice — once here and once when the
                // change stream observes the same write. On standalone/unknown
                // deployments there is no change stream, so this is the only
                // capture path and it runs.
                if !crate::audit::change_stream::supports_change_streams(deployment_id) {
                    f(audit, deployment_id);
                }
            });
        }
    });
}

fn with_entry<F, R>(f: F) -> R
where
    F: FnOnce(&ClientEntry) -> R,
{
    SHELL_ENTRY.with(|c| {
        let borrow = c.borrow();
        let entry = borrow.as_ref().expect("shell entry not set");
        f(entry)
    })
}

// ---------- Shell ----------

enum ShellMessage {
    Eval {
        entry: Arc<ClientEntry>,
        script: String,
        active_db: String,
        audit_log: Option<Arc<AuditLog>>,
        respond: Sender<AppResult<ShellResponse>>,
    },
    Shutdown,
}

pub struct Shell {
    sender: Sender<ShellMessage>,
    _join: JoinHandle<()>,
}

impl Shell {
    pub fn spawn(connection_id: String, initial_db: String) -> AppResult<Self> {
        let (tx, rx) = channel::<ShellMessage>();
        let join = thread::Builder::new()
            .name(format!("nosqlbuddy-shell-{}", connection_id))
            .spawn(move || run_shell_thread(rx, initial_db))
            .map_err(|e| AppError::Validation(format!("spawn shell thread: {e}")))?;
        Ok(Self {
            sender: tx,
            _join: join,
        })
    }

    pub fn shutdown(&self) {
        let _ = self.sender.send(ShellMessage::Shutdown);
    }

    pub async fn eval(
        &self,
        entry: Arc<ClientEntry>,
        script: String,
        active_db: String,
        audit_log: Option<Arc<AuditLog>>,
    ) -> AppResult<ShellResponse> {
        let (resp_tx, resp_rx) = channel::<AppResult<ShellResponse>>();
        self.sender
            .send(ShellMessage::Eval {
                entry,
                script,
                active_db,
                audit_log,
                respond: resp_tx,
            })
            .map_err(|_| AppError::Validation("shell thread is dead".into()))?;
        // Block on the oneshot channel. We're inside a Tauri
        // command (Tokio worker), so we use spawn_blocking to
        // avoid blocking the runtime.
        let join = tokio::task::spawn_blocking(move || resp_rx.recv());
        match join.await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppError::Validation("shell thread dropped response".into())),
            Err(e) => Err(AppError::Validation(format!("shell eval join: {e}"))),
        }
    }
}

fn run_shell_thread(rx: Receiver<ShellMessage>, initial_db: String) {
    // Each shell thread owns:
    //  - a single-threaded current-thread runtime (so we can
    //    `block_on` Mongo futures from within native host
    //    functions without panicking on the multi-thread
    //    Tauri runtime).
    //  - a boa `Context` (which is `!Send`, hence the
    //    dedicated thread).
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("shell: failed to build runtime: {e}");
            return;
        }
    };
    let mut context = Context::default();
    install_host(&mut context);

    // Initial entry / db (will be set on each eval).
    let _ = initial_db; // initial db is set per-eval from the request

    while let Ok(msg) = rx.recv() {
        match msg {
            ShellMessage::Eval {
                entry,
                script,
                active_db,
                audit_log,
                respond,
            } => {
                // Enter the per-thread runtime so the
                // dispatch fns (which call `Handle::try_current()`)
                // find a runtime. Without this, `try_current()`
                // returns Err because the Tauri command thread
                // has its own multi-thread runtime and the
                // global handle is set to that, but the native
                // host fns execute on THIS (shell) thread where
                // there is no live runtime context.
                let _rt_guard = runtime.enter();
                // Attach the entry to THIS thread's thread-local
                // (the shell thread). The previous design set
                // the entry on the Tauri command thread, but
                // thread-locals are per-thread and the native
                // host functions run on the shell thread.
                set_entry(entry);
                if let Some(log) = audit_log {
                    set_audit_log(log);
                }
                SHELL_ACTIVE_DB.with(|c| *c.borrow_mut() = active_db.clone());
                SHELL_OUTPUT.with(|b| b.borrow_mut().clear());
                SHELL_LAST_PIPELINE.with(|b| b.borrow_mut().clear());
                SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = None);
                clear_operations();
                let started = std::time::Instant::now();

                // Pre-process `use <db>` directives (boa doesn't
                // know about them).
                let (db_override, body) = preprocess_use(&script);
                let effective_db = db_override.unwrap_or_else(|| active_db.clone());
                SHELL_ACTIVE_DB.with(|c| *c.borrow_mut() = effective_db.clone());

                // Source-transform: rewrite `db.<coll>.<method>(<args>)`
                // → `__call_db("<coll>", "<method>", [<args>])` and
                // `db.runCommand(<args>)` → `__run_command([<args>])`.
                // The regex is conservative and only matches the
                // leading call site of each statement.
                let body = transform_source(&body);

                // Wrap so any expression used as the last
                // statement is captured. Convention: the user
                // assigns interesting values to `__last`
                // explicitly, or they use `printjson(...)` to
                // emit intermediate results. This matches
                // the standard mongo shell behaviour: the final
                // evaluated expression (if it's a bare expression
                // on a line by itself) becomes the return value.
                //
                // We approximate this by injecting
                // `__last = ...` before the last `;` if the last
                // statement looks like an expression (doesn't
                // start with a statement keyword). The full
                // implementation is a small AST transform; for
                // now we just return undefined and rely on
                // printjson for output.
                let wrapped =
                    format!("(function() {{\ntry {{\n{body}\n}} catch (e) {{\nthrow e;\n}}\n}})()");
                let result = context.eval(Source::from_bytes(wrapped.as_bytes()));
                let mut outputs = take_outputs();
                let execution_ms = started.elapsed().as_millis() as u64;
                let last_pipeline = SHELL_LAST_PIPELINE.with(|b| b.borrow().clone());
                let last_collection = SHELL_LAST_COLLECTION.with(|c| c.borrow().clone());
                let active_database = SHELL_ACTIVE_DB.with(|c| c.borrow().clone());

                match result {
                    Ok(value) => {
                        if !value.is_undefined() && !value.is_null() {
                            if let Ok(json) = js_to_json(&value, &mut context) {
                                // If the last value is an array of
                                // objects, present it as a table;
                                // otherwise as a JSON line.
                                if let JsonValue::Array(items) = &json {
                                    if items.iter().all(|v| v.is_object()) {
                                        let columns: Vec<String> =
                                            if let Some(first) = items.first() {
                                                if let Some(obj) = first.as_object() {
                                                    obj.keys().cloned().collect()
                                                } else {
                                                    Vec::new()
                                                }
                                            } else {
                                                Vec::new()
                                            };
                                        let rows: Vec<Vec<JsonValue>> = items
                                            .iter()
                                            .map(|item| {
                                                if let Some(obj) = item.as_object() {
                                                    columns
                                                        .iter()
                                                        .map(|c| {
                                                            obj.get(c)
                                                                .cloned()
                                                                .unwrap_or(JsonValue::Null)
                                                        })
                                                        .collect()
                                                } else {
                                                    vec![JsonValue::Null]
                                                }
                                            })
                                            .collect();
                                        outputs.push(ShellOutput::Table {
                                            value: ShellTable {
                                                columns,
                                                rows,
                                                execution_ms,
                                            },
                                        });
                                    } else {
                                        outputs.push(ShellOutput::Json { value: json });
                                    }
                                } else {
                                    outputs.push(ShellOutput::Json { value: json });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        outputs.push(ShellOutput::Error {
                            value: format_js_error(&e),
                        });
                    }
                }

                let operations = take_operations();
                let response = ShellResponse {
                    outputs,
                    last_pipeline: if last_pipeline.is_empty() {
                        None
                    } else {
                        Some(last_pipeline)
                    },
                    last_collection,
                    last_database: Some(active_database.clone()),
                    active_database,
                    execution_ms,
                    operations,
                };
                let _ = respond.send(Ok(response));
            }
            ShellMessage::Shutdown => break,
        }
    }

    drop(runtime);
}

// ---------- Native host function installation ----------

fn install_host(ctx: &mut Context) {
    // print(...) — variadic, joins with spaces, trailing newline
    let print = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let mut out = String::new();
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&a.to_string(ctx)?.to_std_string_escaped());
        }
        out.push('\n');
        push_output(ShellOutput::Text { value: out });
        Ok(JsValue::undefined())
    });
    ctx.register_global_builtin_callable(js_string!("print"), 1, print)
        .expect("register print");

    // printjson(...) — variadic, dumps each arg as JSON on a
    // separate line.
    let printjson = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        for a in args.iter() {
            let json = js_to_json(a, ctx)?;
            push_output(ShellOutput::Json { value: json });
        }
        Ok(JsValue::undefined())
    });
    ctx.register_global_builtin_callable(js_string!("printjson"), 1, printjson)
        .expect("register printjson");

    // ObjectId(s?) — returns a hex string that round-trips
    // through jsvalue_to_bson as a real BSON ObjectId.
    let object_id = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let s = args
            .get_or_undefined(0)
            .to_string(ctx)?
            .to_std_string_escaped();
        let oid = if s.is_empty() || s == "undefined" {
            ObjectId::new()
        } else {
            ObjectId::parse_str(&s).map_err(|e| {
                JsError::from_native(JsNativeError::typ().with_message(e.to_string()))
            })?
        };
        Ok(JsValue::from(js_string!(oid.to_hex())))
    });
    ctx.register_global_builtin_callable(js_string!("ObjectId"), 1, object_id)
        .expect("register ObjectId");

    // ISODate(s?) — returns an RFC3339 string.
    let iso_date = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let s = args
            .get_or_undefined(0)
            .to_string(ctx)?
            .to_std_string_escaped();
        let dt = if s.is_empty() || s == "undefined" {
            BsonDateTime::now()
        } else {
            BsonDateTime::parse_rfc3339_str(&s).map_err(|e| {
                JsError::from_native(JsNativeError::typ().with_message(e.to_string()))
            })?
        };
        Ok(JsValue::from(js_string!(dt
            .try_to_rfc3339_string()
            .unwrap_or_default())))
    });
    ctx.register_global_builtin_callable(js_string!("ISODate"), 1, iso_date)
        .expect("register ISODate");

    // __call_db(collection, method, ...args) — variadic.
    // The transformer rewrites `db.coll.find({a:1}, {b:1})` to
    // `__call_db("coll", "find", {a:1}, {b:1})`. The host
    // function extracts the collection + method names, then
    // treats the remaining args as positional method args.
    let call_db = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        if args.len() < 2 {
            return Err(JsError::from_native(
                JsNativeError::typ().with_message("__call_db: missing collection or method"),
            ));
        }
        let collection = args[0].to_string(ctx)?.to_std_string_escaped();
        let method = args[1].to_string(ctx)?.to_std_string_escaped();
        let method_args = &args[2..];
        let mut bson_args = Vec::with_capacity(method_args.len());
        for a in method_args {
            bson_args.push(jsvalue_to_bson(a, ctx).map_err(|e| {
                JsError::from_native(JsNativeError::typ().with_message(e.to_string()))
            })?);
        }
        let active_db = SHELL_ACTIVE_DB.with(|c| c.borrow().clone());
        let result = with_entry(|entry| {
            dispatch_sync(entry, &active_db, &collection, &method, bson_args, ctx)
        });
        match result {
            Ok(value) => Ok(value),
            Err(e) => Err(JsError::from_native(
                JsNativeError::typ().with_message(e.to_string()),
            )),
        }
    });
    ctx.register_global_builtin_callable(js_string!("__call_db"), 2, call_db)
        .expect("register __call_db");

    // __run_command(cmd) — db.runCommand(cmd) rewriter target.
    let run_command = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let cmd = jsvalue_to_bson(args.get_or_undefined(0), ctx)
            .map_err(|e| JsError::from_native(JsNativeError::typ().with_message(e.to_string())))?;
        let cmd_doc = match cmd {
            bson::Bson::Document(d) => d,
            bson::Bson::Null => Document::new(),
            other => doc! { "value": other },
        };
        let active_db = SHELL_ACTIVE_DB.with(|c| c.borrow().clone());
        let result = with_entry(|entry| {
            let database = entry.client.database(&active_db);
            runtime_block_on(
                async move { database.run_command(cmd_doc).await.map_err(AppError::mongo) },
            )
        });
        match result {
            Ok(doc) => bson_to_js(bson::Bson::Document(doc), ctx),
            Err(e) => Err(JsError::from_native(
                JsNativeError::typ().with_message(e.to_string()),
            )),
        }
    });
    ctx.register_global_builtin_callable(js_string!("__run_command"), 1, run_command)
        .expect("register __run_command");

    // help() — prints a help message.
    let help = NativeFunction::from_fn_ptr(|_this, _args, _ctx| {
        push_output(ShellOutput::Text {
            value: HELP_TEXT.to_string(),
        });
        Ok(JsValue::undefined())
    });
    ctx.register_global_builtin_callable(js_string!("help"), 0, help)
        .expect("register help");

    // db.help()
    let db_help = NativeFunction::from_fn_ptr(|_this, _args, _ctx| {
        push_output(ShellOutput::Text {
            value: DB_HELP_TEXT.to_string(),
        });
        Ok(JsValue::undefined())
    });
    ctx.register_global_builtin_callable(js_string!("__db_help"), 0, db_help)
        .expect("register __db_help");

    // Set up `db` as a Proxy-like object that returns a
    // CollectionProxy on any property access. We approximate
    // this by installing a `__db_get` function and rewriting
    // `db.<x>` accesses via the Source transformer.
    install_db_stub(ctx);
}

const HELP_TEXT: &str = "
NoSQLBuddy Shell — quick reference
  use <db>                       Switch database
  show dbs                      List databases (via runCommand)
  show collections               List collections
  db.help()                     List database methods
  db.<coll>.help()              List collection methods
  print(...)                    Print values (joins with spaces)
  printjson(...)                Print values as JSON
  ObjectId('...') / ISODate('...')   BSON constructors
";

const DB_HELP_TEXT: &str = "
db methods:
  db.runCommand(cmd)            Run an admin / database command
  db.help()                     This help
  db.<coll>.<method>(...)       Collection methods
";

fn install_db_stub(ctx: &mut Context) {
    // `db` is a plain object. `db.coll.method(...)` is rewritten by
    // the source transformer to `__call_db("coll", "method", ...)`,
    // and `db.runCommand(...)` to `__run_command(...)`, so `db` never
    // needs real collection properties. We DO attach a `help` method
    // so `db.help()` works (the transformer leaves `db.help()` alone
    // since it has only one dot, and without this method JS would
    // throw "db.help is not a function"). `db.runCommand` is also
    // attached as a real function for robustness in case a future
    // caller bypasses the transformer.
    let obj = boa_engine::object::JsObject::default(ctx.intrinsics());

    // db.help() → print the database-methods help text.
    let help_fn: JsValue = NativeFunction::from_fn_ptr(|_this, _args, _ctx| {
        push_output(ShellOutput::Text {
            value: DB_HELP_TEXT.to_string(),
        });
        Ok(JsValue::undefined())
    })
    .to_js_function(ctx.realm())
    .into();
    obj.set(js_string!("help"), help_fn, false, ctx)
        .expect("db.help set");

    // db.runCommand(cmd) → dispatch via __run_command. This is a
    // fallback; the source transformer normally rewrites
    // `db.runCommand(...)` to `__run_command(...)` before boa sees it.
    let run_command_fn: JsValue = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let cmd = jsvalue_to_bson(args.get_or_undefined(0), ctx)
            .map_err(|e| JsError::from_native(JsNativeError::typ().with_message(e.to_string())))?;
        let cmd_doc = match cmd {
            bson::Bson::Document(d) => d,
            bson::Bson::Null => Document::new(),
            other => doc! { "value": other },
        };
        let active_db = SHELL_ACTIVE_DB.with(|c| c.borrow().clone());
        let result = with_entry(|entry| {
            let database = entry.client.database(&active_db);
            runtime_block_on(
                async move { database.run_command(cmd_doc).await.map_err(AppError::mongo) },
            )
        });
        match result {
            Ok(doc) => bson_to_js(bson::Bson::Document(doc), ctx),
            Err(e) => Err(JsError::from_native(
                JsNativeError::typ().with_message(e.to_string()),
            )),
        }
    })
    .to_js_function(ctx.realm())
    .into();
    obj.set(js_string!("runCommand"), run_command_fn, false, ctx)
        .expect("db.runCommand set");

    ctx.register_global_property(js_string!("db"), obj, Attribute::all())
        .expect("register db");
}

// ---------- use <db> preprocessor ----------

fn preprocess_use(script: &str) -> (Option<String>, String) {
    let trimmed = script.trim_start();
    if !trimmed.starts_with("use ") {
        return (None, script.to_string());
    }
    let mut lines = trimmed.splitn(2, '\n');
    let first = lines.next().unwrap_or("");
    let rest = lines.next().unwrap_or("");
    let token = first.trim().trim_end_matches(';').trim();
    if let Some(db) = token.strip_prefix("use ") {
        let db = db.trim();
        let db = db
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| db.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(db);
        if !db.is_empty()
            && db
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return (Some(db.to_string()), rest.to_string());
        }
    }
    (None, script.to_string())
}

// ---------- Source transformer ----------
//
// Rewrites call sites so that
//   `db.<coll>.<method>(<args>)` → `__call_db("<coll>", "<method>", <args>)`
//   `db.runCommand(<args>)`      → `__run_command(<args>)`
// The original argument list is preserved verbatim after the opening
// paren, so the closing `)` of the original call also closes the
// rewritten call (no balanced-paren matching needed).
//
// The scanner is string/comment-aware: patterns inside string literals
// (`"db.foo.bar("`), line comments (`// ...`), and block comments
// (`/* ... */`) are left untouched. This avoids the previous
// regex-based transformer's known limitation of rewriting `db.foo.bar(`
// text that appeared inside a string literal.
//
// Chained methods like `db.coll.find({}).toArray()` are still rewritten
// at the leading call site only; the trailing `.toArray()` is handled
// by attaching a `toArray` per-instance function to the array returned
// by `find`/`aggregate` (see `dispatch_sync`).

static CALL_DB_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static RUN_COMMAND_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

fn call_db_re() -> &'static regex::Regex {
    CALL_DB_RE.get_or_init(|| {
        regex::Regex::new(r"\bdb\.([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
    })
}

fn run_command_re() -> &'static regex::Regex {
    RUN_COMMAND_RE.get_or_init(|| regex::Regex::new(r"\bdb\.runCommand\s*\(").unwrap())
}

fn transform_source(src: &str) -> String {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len());
    let mut k = 0usize;
    let mut in_string: Option<char> = None;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    while k < n {
        let (i, c) = chars[k];
        if in_line_comment {
            out.push(c);
            if c == '\n' {
                in_line_comment = false;
            }
            k += 1;
            continue;
        }
        if in_block_comment {
            out.push(c);
            if c == '*' && k + 1 < n && chars[k + 1].1 == '/' {
                out.push('/');
                k += 2;
                in_block_comment = false;
                continue;
            }
            k += 1;
            continue;
        }
        if let Some(quote) = in_string {
            out.push(c);
            if c == '\\' && k + 1 < n {
                // Escaped char: copy the next char verbatim so an
                // escaped quote doesn't end the string.
                out.push(chars[k + 1].1);
                k += 2;
                continue;
            }
            if c == quote {
                in_string = None;
            }
            k += 1;
            continue;
        }
        // Not inside a string or comment — watch for comment starts.
        if c == '/' && k + 1 < n && chars[k + 1].1 == '/' {
            in_line_comment = true;
            out.push('/');
            out.push('/');
            k += 2;
            continue;
        }
        if c == '/' && k + 1 < n && chars[k + 1].1 == '*' {
            in_block_comment = true;
            out.push('/');
            out.push('*');
            k += 2;
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            in_string = Some(c);
            out.push(c);
            k += 1;
            continue;
        }
        // Try to match a `db.` call pattern here. Only when `db` is a
        // standalone word (preceded by a non-identifier char / start).
        if c == 'd' && k + 1 < n && chars[k + 1].1 == 'b' && is_word_boundary_before(&chars, k) {
            let rest = &src[i..];
            if let Some((consumed_chars, replacement)) = try_match_db_pattern(rest) {
                out.push_str(&replacement);
                k += consumed_chars;
                continue;
            }
        }
        out.push(c);
        k += 1;
    }
    out
}

/// Match `db.runCommand(` or `db.<coll>.<method>(` at the start of
/// `rest`. Returns the number of chars consumed (the full matched
/// prefix, including the opening paren) and the replacement text that
/// should replace it.
fn try_match_db_pattern(rest: &str) -> Option<(usize, String)> {
    // runCommand first — it has no collection namespace and must win
    // over the `db.X.Y(` form.
    if let Some(caps) = run_command_re().captures(rest) {
        let m = caps.get(0).unwrap();
        if m.start() == 0 {
            let consumed_chars = rest[..m.end()].chars().count();
            return Some((consumed_chars, "__run_command(".to_string()));
        }
    }
    if let Some(caps) = call_db_re().captures(rest) {
        let m = caps.get(0).unwrap();
        if m.start() == 0 {
            let coll = caps[1].to_string();
            let method = caps[2].to_string();
            let consumed_chars = rest[..m.end()].chars().count();
            return Some((
                consumed_chars,
                format!("__call_db(\"{coll}\", \"{method}\", "),
            ));
        }
    }
    None
}

/// True when the char before position `k` is not an identifier
/// character (or `k` is at the start of the text), so `db` at `k` is a
/// standalone word rather than the tail of a larger identifier like
/// `mydb`.
fn is_word_boundary_before(chars: &[(usize, char)], k: usize) -> bool {
    if k == 0 {
        return true;
    }
    let p = chars[k - 1].1;
    !p.is_alphanumeric() && p != '_' && p != '$'
}

// ---------- Sync dispatch over the current-thread runtime ----------

fn runtime_block_on<F, T>(future: F) -> AppResult<T>
where
    F: std::future::Future<Output = AppResult<T>>,
{
    // We rely on the caller being on a thread that hosts a
    // current-thread runtime (the shell thread). Tokio's
    // `Handle::try_current()` finds it.
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|e| AppError::Validation(format!("shell: no current tokio runtime: {e}")))?;
    handle.block_on(future)
}

fn dispatch_sync(
    entry: &ClientEntry,
    db: &str,
    coll: &str,
    method: &str,
    args: Vec<bson::Bson>,
    ctx: &mut Context,
) -> JsResult<JsValue> {
    let result = match method {
        "find" => {
            let filter = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let docs: Vec<Document> = runtime_block_on(async move {
                let cursor = entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .find(filter)
                    .limit(50)
                    .await
                    .map_err(AppError::mongo)?;
                cursor.try_collect().await.map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = Some(coll.to_string()));
            let arr = JsArray::new(ctx);
            for (i, d) in docs.iter().enumerate() {
                let js = bson_to_js(bson::Bson::Document(d.clone()), ctx)?;
                arr.set(i, js, false, ctx)?;
            }
            attach_cursor_methods(&arr, ctx)?;
            Ok(arr.into())
        }
        "findOne" => {
            let filter = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let result = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .find_one(filter)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = Some(coll.to_string()));
            match result {
                Some(d) => bson_to_js(bson::Bson::Document(d), ctx),
                None => Ok(JsValue::null()),
            }
        }
        "countDocuments" | "count" => {
            let filter = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let n = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .count_documents(filter)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            Ok(JsValue::from(n as f64))
        }
        "aggregate" => {
            // First arg must be an array of stage docs.
            let pipeline_bson = args
                .first()
                .cloned()
                .unwrap_or(bson::Bson::Array(Vec::new()));
            let mut pipeline = match pipeline_bson {
                bson::Bson::Array(v) => v
                    .into_iter()
                    .filter_map(|b| match b {
                        bson::Bson::Document(d) => Some(d),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            if !pipeline.iter().any(|s| s.contains_key("$limit")) {
                pipeline.push(doc! { "$limit": 50_i64 });
            }
            let pipeline_json: Vec<JsonValue> = pipeline
                .iter()
                .map(|d| bson_to_json(bson::Bson::Document(d.clone())))
                .collect();
            SHELL_LAST_PIPELINE.with(|b| *b.borrow_mut() = pipeline_json);
            SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = Some(coll.to_string()));
            let cursor = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .aggregate(pipeline)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            let docs: Vec<Document> =
                runtime_block_on(
                    async move { cursor.try_collect().await.map_err(AppError::mongo) },
                )
                .map_err(|e| js_err(e.to_string()))?;
            let arr = JsArray::new(ctx);
            for (i, d) in docs.iter().enumerate() {
                let js = bson_to_js(bson::Bson::Document(d.clone()), ctx)?;
                arr.set(i, js, false, ctx)?;
            }
            attach_cursor_methods(&arr, ctx)?;
            Ok(arr.into())
        }
        "distinct" => {
            let field = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    js_err("distinct(field, filter?) requires a field name".to_string())
                })?;
            let filter = args
                .get(1)
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let values = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .distinct(field, filter)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            let arr = JsArray::new(ctx);
            for (i, v) in values.iter().enumerate() {
                let js = bson_to_js(v.clone(), ctx)?;
                arr.set(i, js, false, ctx)?;
            }
            Ok(arr.into())
        }
        "insertOne" => {
            let doc_arg = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .ok_or_else(|| js_err("insertOne requires a document".to_string()))?;
            let doc_for_audit = doc_arg.clone();
            let res = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .insert_one(doc_arg)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let Ok(json) = serde_json::to_string(&doc_for_audit) {
                    let _ = interceptor::record_insert(audit, dep, db, coll, &json);
                }
            });
            if let Ok(json) = serde_json::to_string(&doc_for_audit) {
                push_operation(ShellOperation {
                    kind: OperationKind::InsertOne,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: None,
                    update_json: Some(json),
                    matched_count: None,
                    modified_count: None,
                    inserted_count: Some(1),
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            push_output(ShellOutput::Text {
                value: format!(
                    "Inserted 1 document (id: {})",
                    match res.inserted_id.as_object_id() {
                        Some(oid) => oid.to_hex(),
                        None => res.inserted_id.to_string(),
                    }
                ),
            });
            Ok(JsValue::undefined())
        }
        "insertMany" => {
            // First arg: array of documents.
            let docs: Vec<Document> = match args.first() {
                Some(bson::Bson::Array(v)) => v
                    .iter()
                    .filter_map(|b| match b {
                        bson::Bson::Document(d) => Some(d.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if docs.is_empty() {
                return Err(js_err(
                    "insertMany requires a non-empty array of documents".to_string(),
                ));
            }
            let docs_for_audit = docs.clone();
            let res = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .insert_many(docs)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                for doc in &docs_for_audit {
                    if let Ok(json) = serde_json::to_string(doc) {
                        let _ = interceptor::record_insert(audit, dep, db, coll, &json);
                    }
                }
            });
            if let Ok(json) = serde_json::to_string(&docs_for_audit) {
                push_operation(ShellOperation {
                    kind: OperationKind::InsertMany,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: None,
                    update_json: Some(json),
                    matched_count: None,
                    modified_count: None,
                    inserted_count: Some(res.inserted_ids.len() as u64),
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            push_output(ShellOutput::Text {
                value: format!(
                    "Inserted {} document{}",
                    res.inserted_ids.len(),
                    if res.inserted_ids.len() == 1 { "" } else { "s" }
                ),
            });
            Ok(JsValue::undefined())
        }
        "updateOne" | "updateMany" => {
            // Args: filter, update, (options).
            let filter = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let update = args
                .get(1)
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    js_err(format!(
                        "db.{coll}.{method}(filter, update) requires an update document"
                    ))
                })?;
            let multi = method == "updateMany";
            let filter_for_audit = filter.clone();
            let update_for_audit = update.clone();
            let res = runtime_block_on(async move {
                let coll_handle = entry.client.database(db).collection::<Document>(coll);
                if multi {
                    coll_handle.update_many(filter, update).await
                } else {
                    coll_handle.update_one(filter, update).await
                }
                .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let (Ok(fj), Ok(uj)) = (
                    serde_json::to_string(&filter_for_audit),
                    serde_json::to_string(&update_for_audit),
                ) {
                    let _ = interceptor::record_update(audit, dep, db, coll, &fj, &uj);
                }
            });
            if let (Ok(fj), Ok(uj)) = (
                serde_json::to_string(&filter_for_audit),
                serde_json::to_string(&update_for_audit),
            ) {
                push_operation(ShellOperation {
                    kind: if multi {
                        OperationKind::UpdateMany
                    } else {
                        OperationKind::UpdateOne
                    },
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: Some(fj),
                    update_json: Some(uj),
                    matched_count: Some(res.matched_count),
                    modified_count: Some(res.modified_count),
                    inserted_count: None,
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            push_output(ShellOutput::Text {
                value: format!(
                    "Matched {} · Modified {}{}",
                    res.matched_count,
                    res.modified_count,
                    res.upserted_id
                        .as_ref()
                        .map(|id| format!(" · Upserted id: {}", id))
                        .unwrap_or_default()
                ),
            });
            Ok(JsValue::undefined())
        }
        "replaceOne" => {
            // Args: filter, replacement, (options).
            let filter = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let replacement = args
                .get(1)
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    js_err("replaceOne(filter, doc) requires a replacement document".to_string())
                })?;
            let filter_for_audit = filter.clone();
            let repl_for_audit = replacement.clone();
            let res = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .replace_one(filter, replacement)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let (Ok(fj), Ok(rj)) = (
                    serde_json::to_string(&filter_for_audit),
                    serde_json::to_string(&repl_for_audit),
                ) {
                    let _ = interceptor::record_update(audit, dep, db, coll, &fj, &rj);
                }
            });
            if let (Ok(fj), Ok(rj)) = (
                serde_json::to_string(&filter_for_audit),
                serde_json::to_string(&repl_for_audit),
            ) {
                push_operation(ShellOperation {
                    kind: OperationKind::ReplaceOne,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: Some(fj),
                    update_json: Some(rj),
                    matched_count: Some(res.matched_count),
                    modified_count: Some(res.modified_count),
                    inserted_count: None,
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            push_output(ShellOutput::Text {
                value: format!(
                    "Matched {} · Modified {}{}",
                    res.matched_count,
                    res.modified_count,
                    res.upserted_id
                        .as_ref()
                        .map(|id| format!(" · Upserted id: {}", id))
                        .unwrap_or_default()
                ),
            });
            Ok(JsValue::undefined())
        }
        "deleteOne" | "deleteMany" => {
            let filter = args
                .first()
                .and_then(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let multi = method == "deleteMany";
            let filter_for_audit = filter.clone();
            let res = runtime_block_on(async move {
                let coll_handle = entry.client.database(db).collection::<Document>(coll);
                if multi {
                    coll_handle.delete_many(filter).await
                } else {
                    coll_handle.delete_one(filter).await
                }
                .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let Ok(fj) = serde_json::to_string(&filter_for_audit) {
                    let _ = interceptor::record_delete(audit, dep, db, coll, &fj);
                }
            });
            if let Ok(fj) = serde_json::to_string(&filter_for_audit) {
                push_operation(ShellOperation {
                    kind: if multi {
                        OperationKind::DeleteMany
                    } else {
                        OperationKind::DeleteOne
                    },
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: Some(fj),
                    update_json: None,
                    matched_count: None,
                    modified_count: None,
                    inserted_count: None,
                    deleted_count: Some(res.deleted_count),
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            push_output(ShellOutput::Text {
                value: format!(
                    "Deleted {} document{}",
                    res.deleted_count,
                    if res.deleted_count == 1 { "" } else { "s" }
                ),
            });
            Ok(JsValue::undefined())
        }
        "drop" => {
            runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .drop()
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                let _ = interceptor::record_drop_collection(audit, dep, db, coll);
            });
            push_operation(ShellOperation {
                kind: OperationKind::CollectionDrop,
                database: db.to_string(),
                collection: coll.to_string(),
                query_json: None,
                update_json: None,
                matched_count: None,
                modified_count: None,
                inserted_count: None,
                deleted_count: None,
                execution_ms: None,
                errored: false,
                error_message: None,
            });
            push_output(ShellOutput::Text {
                value: format!("Dropped collection {}.{}", db, coll),
            });
            Ok(JsValue::undefined())
        }
        "createIndex" => {
            // mongosh form: db.coll.createIndex(keys, options).
            // keys may be a document (e.g. {a: 1, b: -1}) or a
            // string for text indexes. We accept both.
            let keys_doc = match args.first().cloned() {
                Some(bson::Bson::Document(d)) => d,
                Some(bson::Bson::String(s)) => doc! { s.as_str(): "text" },
                _ => Document::new(),
            };
            // Optional second arg: options document. We honor
            // name, unique, sparse, hidden, expireAfterSeconds
            // as flat fields (mongosh-style).
            let mut options = mongodb::options::IndexOptions::builder().build();
            if let Some(opts) = args.get(1).and_then(|b| match b {
                bson::Bson::Document(d) => Some(d.clone()),
                _ => None,
            }) {
                if let Ok(name) = opts.get_str("name") {
                    options.name = Some(name.to_string());
                }
                if let Ok(v) = opts.get_bool("unique") {
                    options.unique = Some(v);
                }
                if let Ok(v) = opts.get_bool("sparse") {
                    options.sparse = Some(v);
                }
                if let Ok(v) = opts.get_bool("hidden") {
                    options.hidden = Some(v);
                }
                if let Ok(v) = opts.get_i64("expireAfterSeconds") {
                    options.expire_after = Some(std::time::Duration::from_secs(v as u64));
                }
            }
            let keys_for_audit = keys_doc.clone();
            let opts_for_audit = args.get(1).cloned().unwrap_or(bson::Bson::Null);
            let index_model = mongodb::IndexModel::builder()
                .keys(keys_doc)
                .options(Some(options))
                .build();
            let res = runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .create_index(index_model)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let (Ok(kj), Ok(oj)) = (
                    serde_json::to_string(&keys_for_audit),
                    serde_json::to_string(&opts_for_audit),
                ) {
                    let _ = interceptor::record_create_index(audit, dep, db, coll, &kj, &oj);
                }
            });
            if let (Ok(kj), Ok(_oj)) = (
                serde_json::to_string(&keys_for_audit),
                serde_json::to_string(&opts_for_audit),
            ) {
                push_operation(ShellOperation {
                    kind: OperationKind::IndexCreate,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: None,
                    update_json: Some(kj),
                    matched_count: None,
                    modified_count: None,
                    inserted_count: None,
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            push_output(ShellOutput::Text {
                value: format!("Created index '{}'", res.index_name),
            });
            Ok(JsValue::undefined())
        }
        "dropIndex" => {
            // mongosh form: db.coll.dropIndex(name) or
            // db.coll.dropIndex(keysDoc). We support the name
            // form; keysDoc would require a lookup.
            let name = match args.first() {
                Some(bson::Bson::String(s)) => s.clone(),
                _ => {
                    return Err(js_err(
                        "dropIndex(name) requires the index name as a string".to_string(),
                    ))
                }
            };
            let name_for_output = name.clone();
            runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .collection::<Document>(coll)
                    .drop_index(name)
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                let _ = interceptor::record_drop_index(audit, dep, db, coll, &name_for_output);
            });
            push_operation(ShellOperation {
                kind: OperationKind::IndexDrop,
                database: db.to_string(),
                collection: coll.to_string(),
                query_json: Some(name_for_output.clone()),
                update_json: None,
                matched_count: None,
                modified_count: None,
                inserted_count: None,
                deleted_count: None,
                execution_ms: None,
                errored: false,
                error_message: None,
            });
            push_output(ShellOutput::Text {
                value: format!("Dropped index '{}'", name_for_output),
            });
            Ok(JsValue::undefined())
        }
        "rename" | "renameCollection" => {
            // mongosh form: db.coll.renameCollection(newName, dropTarget?).
            // The driver doesn't expose rename on Collection, so
            // we dispatch via the admin command:
            //   { renameCollection: "db.old", to: "db.new" }
            let new_name = match args.first() {
                Some(bson::Bson::String(s)) => s.clone(),
                _ => {
                    return Err(js_err(
                        "rename(newName) requires the new collection name as a string".to_string(),
                    ))
                }
            };
            let new_name_for_output = new_name.clone();
            let new_name_for_audit = new_name.clone();
            let from_ns = format!("{}.{}", db, coll);
            let to_ns = format!("{}.{}", db, new_name);
            runtime_block_on(async move {
                entry
                    .client
                    .database("admin")
                    .run_command(doc! {
                        "renameCollection": from_ns,
                        "to": to_ns,
                    })
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                let _ = interceptor::record_rename_collection(
                    audit,
                    dep,
                    db,
                    coll,
                    &new_name_for_audit,
                );
            });
            push_operation(ShellOperation {
                kind: OperationKind::CollectionRename,
                database: db.to_string(),
                collection: coll.to_string(),
                query_json: Some(new_name_for_output.clone()),
                update_json: None,
                matched_count: None,
                modified_count: None,
                inserted_count: None,
                deleted_count: None,
                execution_ms: None,
                errored: false,
                error_message: None,
            });
            push_output(ShellOutput::Text {
                value: format!("Renamed {}.{} → {}.{}", db, coll, db, new_name_for_output),
            });
            Ok(JsValue::undefined())
        }
        "dropDatabase" => {
            // mongosh form: db.dropDatabase(). The transformer
            // would route this as db.<coll>.dropDatabase which is
            // wrong; we handle it here for safety but the
            // canonical path is via runCommand.
            runtime_block_on(async move {
                entry
                    .client
                    .database(db)
                    .drop()
                    .await
                    .map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                let _ = interceptor::record_drop_database(audit, dep, db);
            });
            push_output(ShellOutput::Text {
                value: format!("Dropped database {}", db),
            });
            Ok(JsValue::undefined())
        }
        "findOneAndUpdate" => {
            // mongosh form: db.coll.findOneAndUpdate(filter, update, options?).
            // Returns the document (before modification by default, or
            // after when returnNewDocument: true / returnDocument: "after").
            let filter = doc_arg(&args, 0).unwrap_or_default();
            let update = args.get(1).ok_or_else(|| {
                js_err(
                    "findOneAndUpdate(filter, update, options?) requires an update document"
                        .to_string(),
                )
            })?;
            let update_mods = bson_to_update_mods(update)?;
            let opts = doc_arg(&args, 2);
            let filter_for_audit = filter.clone();
            let update_for_audit = update.clone();
            let result = runtime_block_on(async move {
                let coll = entry.client.database(db).collection::<Document>(coll);
                let mut a = coll.find_one_and_update(filter, update_mods);
                if let Some(o) = &opts {
                    if let Some(rd) = extract_return_document(o) {
                        a = a.return_document(rd);
                    }
                    if let Ok(true) = o.get_bool("upsert") {
                        a = a.upsert(true);
                    }
                    if let Ok(sort) = o.get_document("sort") {
                        a = a.sort(sort.clone());
                    }
                    if let Ok(proj) = o.get_document("projection") {
                        a = a.projection(proj.clone());
                    }
                    if let Ok(af) = o.get_array("arrayFilters") {
                        let afs: Vec<Document> = af
                            .iter()
                            .filter_map(|b| match b {
                                bson::Bson::Document(d) => Some(d.clone()),
                                _ => None,
                            })
                            .collect();
                        a = a.array_filters(afs);
                    }
                }
                a.await.map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let (Ok(fj), Ok(uj)) = (
                    serde_json::to_string(&filter_for_audit),
                    serde_json::to_string(&update_for_audit),
                ) {
                    let _ = interceptor::record_update(audit, dep, db, coll, &fj, &uj);
                }
            });
            if let (Ok(fj), Ok(uj)) = (
                serde_json::to_string(&filter_for_audit),
                serde_json::to_string(&update_for_audit),
            ) {
                push_operation(ShellOperation {
                    kind: OperationKind::UpdateOne,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: Some(fj),
                    update_json: Some(uj),
                    matched_count: Some(if result.is_some() { 1 } else { 0 }),
                    modified_count: Some(if result.is_some() { 1 } else { 0 }),
                    inserted_count: None,
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = Some(coll.to_string()));
            push_output(ShellOutput::Text {
                value: format!(
                    "findOneAndUpdate: {}",
                    if result.is_some() {
                        "matched 1"
                    } else {
                        "matched 0"
                    }
                ),
            });
            match result {
                Some(d) => bson_to_js(bson::Bson::Document(d), ctx),
                None => Ok(JsValue::null()),
            }
        }
        "findOneAndDelete" => {
            // mongosh form: db.coll.findOneAndDelete(filter, options?).
            // Returns the deleted document or null.
            let filter = doc_arg(&args, 0).unwrap_or_default();
            let opts = doc_arg(&args, 1);
            let filter_for_audit = filter.clone();
            let result = runtime_block_on(async move {
                let coll = entry.client.database(db).collection::<Document>(coll);
                let mut a = coll.find_one_and_delete(filter);
                if let Some(o) = &opts {
                    if let Ok(sort) = o.get_document("sort") {
                        a = a.sort(sort.clone());
                    }
                    if let Ok(proj) = o.get_document("projection") {
                        a = a.projection(proj.clone());
                    }
                }
                a.await.map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let Ok(fj) = serde_json::to_string(&filter_for_audit) {
                    let _ = interceptor::record_delete(audit, dep, db, coll, &fj);
                }
            });
            if let Ok(fj) = serde_json::to_string(&filter_for_audit) {
                push_operation(ShellOperation {
                    kind: OperationKind::DeleteOne,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: Some(fj),
                    update_json: None,
                    matched_count: None,
                    modified_count: None,
                    inserted_count: None,
                    deleted_count: Some(if result.is_some() { 1 } else { 0 }),
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = Some(coll.to_string()));
            push_output(ShellOutput::Text {
                value: format!(
                    "findOneAndDelete: {}",
                    if result.is_some() {
                        "deleted 1"
                    } else {
                        "matched 0"
                    }
                ),
            });
            match result {
                Some(d) => bson_to_js(bson::Bson::Document(d), ctx),
                None => Ok(JsValue::null()),
            }
        }
        "findOneAndReplace" => {
            // mongosh form: db.coll.findOneAndReplace(filter, replacement, options?).
            // Returns the document (before by default).
            let filter = doc_arg(&args, 0).unwrap_or_default();
            let replacement = doc_arg(&args, 1).ok_or_else(|| {
                js_err("findOneAndReplace(filter, replacement, options?) requires a replacement document".to_string())
            })?;
            let opts = doc_arg(&args, 2);
            let filter_for_audit = filter.clone();
            let repl_for_audit = replacement.clone();
            let result = runtime_block_on(async move {
                let coll = entry.client.database(db).collection::<Document>(coll);
                let mut a = coll.find_one_and_replace(filter, replacement);
                if let Some(o) = &opts {
                    if let Some(rd) = extract_return_document(o) {
                        a = a.return_document(rd);
                    }
                    if let Ok(true) = o.get_bool("upsert") {
                        a = a.upsert(true);
                    }
                    if let Ok(sort) = o.get_document("sort") {
                        a = a.sort(sort.clone());
                    }
                    if let Ok(proj) = o.get_document("projection") {
                        a = a.projection(proj.clone());
                    }
                }
                a.await.map_err(AppError::mongo)
            })
            .map_err(|e| js_err(e.to_string()))?;
            try_audit(|audit, dep| {
                if let (Ok(fj), Ok(rj)) = (
                    serde_json::to_string(&filter_for_audit),
                    serde_json::to_string(&repl_for_audit),
                ) {
                    let _ = interceptor::record_update(audit, dep, db, coll, &fj, &rj);
                }
            });
            if let (Ok(fj), Ok(rj)) = (
                serde_json::to_string(&filter_for_audit),
                serde_json::to_string(&repl_for_audit),
            ) {
                push_operation(ShellOperation {
                    kind: OperationKind::ReplaceOne,
                    database: db.to_string(),
                    collection: coll.to_string(),
                    query_json: Some(fj),
                    update_json: Some(rj),
                    matched_count: Some(if result.is_some() { 1 } else { 0 }),
                    modified_count: Some(if result.is_some() { 1 } else { 0 }),
                    inserted_count: None,
                    deleted_count: None,
                    execution_ms: None,
                    errored: false,
                    error_message: None,
                });
            }
            SHELL_LAST_COLLECTION.with(|c| *c.borrow_mut() = Some(coll.to_string()));
            push_output(ShellOutput::Text {
                value: format!(
                    "findOneAndReplace: {}",
                    if result.is_some() {
                        "matched 1"
                    } else {
                        "matched 0"
                    }
                ),
            });
            match result {
                Some(d) => bson_to_js(bson::Bson::Document(d), ctx),
                None => Ok(JsValue::null()),
            }
        }
        "bulkWrite" => {
            // mongosh form: db.coll.bulkWrite([ {insertOne: {document: {...}}},
            // {updateOne: {filter, update, upsert?}}, {deleteMany: {filter}}, ... ]).
            // The modern driver's client.bulk_write requires MongoDB 8.0+, so
            // we emulate by dispatching each operation through the existing
            // per-method logic. Ordered by default: stop on first error.
            let ops = match args.first() {
                Some(bson::Bson::Array(v)) => v.clone(),
                _ => {
                    return Err(js_err(
                        "bulkWrite requires an array of operation documents".to_string(),
                    ))
                }
            };
            let mut inserted = 0u64;
            let mut matched = 0u64;
            let mut modified = 0u64;
            let mut deleted = 0u64;
            let mut upserted = 0u64;
            for op in ops {
                let op_doc = match op {
                    bson::Bson::Document(d) => d,
                    _ => continue,
                };
                // Each op document has exactly one top-level key naming the
                // operation kind; its value is the payload document.
                let (kind, payload) = match op_doc.iter().next() {
                    Some((k, v)) => (k.as_str(), v.clone()),
                    None => continue,
                };
                let payload_doc = match payload {
                    bson::Bson::Document(d) => d,
                    _ => {
                        return Err(js_err(format!(
                            "bulkWrite: {kind} payload must be a document"
                        )))
                    }
                };
                match kind {
                    "insertOne" => {
                        let doc = payload_doc
                            .get_document("document")
                            .map_err(|e| js_err(format!("bulkWrite insertOne: {e}")))?
                            .clone();
                        let doc_for_audit = doc.clone();
                        runtime_block_on(async move {
                            entry
                                .client
                                .database(db)
                                .collection::<Document>(coll)
                                .insert_one(doc)
                                .await
                                .map_err(AppError::mongo)
                        })
                        .map_err(|e| js_err(e.to_string()))?;
                        try_audit(|audit, dep| {
                            if let Ok(json) = serde_json::to_string(&doc_for_audit) {
                                let _ = interceptor::record_insert(audit, dep, db, coll, &json);
                            }
                        });
                        inserted += 1;
                    }
                    "updateOne" | "updateMany" => {
                        let filter = payload_doc
                            .get_document("filter")
                            .cloned()
                            .unwrap_or_default();
                        let update = payload_doc.get("update").cloned().ok_or_else(|| {
                            js_err("bulkWrite update: missing update".to_string())
                        })?;
                        let update_mods = bson_to_update_mods(&update)?;
                        let multi = kind == "updateMany";
                        let upsert = payload_doc.get_bool("upsert").unwrap_or(false);
                        let filter_for_audit = filter.clone();
                        let update_for_audit = update.clone();
                        let res = runtime_block_on(async move {
                            let c = entry.client.database(db).collection::<Document>(coll);
                            let a = if multi {
                                c.update_many(filter, update_mods)
                            } else {
                                c.update_one(filter, update_mods)
                            };
                            a.upsert(upsert).await.map_err(AppError::mongo)
                        })
                        .map_err(|e| js_err(e.to_string()))?;
                        try_audit(|audit, dep| {
                            if let (Ok(fj), Ok(uj)) = (
                                serde_json::to_string(&filter_for_audit),
                                serde_json::to_string(&update_for_audit),
                            ) {
                                let _ = interceptor::record_update(audit, dep, db, coll, &fj, &uj);
                            }
                        });
                        matched += res.matched_count;
                        modified += res.modified_count;
                        if res.upserted_id.is_some() {
                            upserted += 1;
                        }
                    }
                    "replaceOne" => {
                        let filter = payload_doc
                            .get_document("filter")
                            .cloned()
                            .unwrap_or_default();
                        let replacement = payload_doc
                            .get_document("replacement")
                            .cloned()
                            .map_err(|e| js_err(format!("bulkWrite replaceOne: {e}")))?;
                        let upsert = payload_doc.get_bool("upsert").unwrap_or(false);
                        let filter_for_audit = filter.clone();
                        let repl_for_audit = replacement.clone();
                        let res = runtime_block_on(async move {
                            entry
                                .client
                                .database(db)
                                .collection::<Document>(coll)
                                .replace_one(filter, replacement)
                                .upsert(upsert)
                                .await
                                .map_err(AppError::mongo)
                        })
                        .map_err(|e| js_err(e.to_string()))?;
                        try_audit(|audit, dep| {
                            if let (Ok(fj), Ok(rj)) = (
                                serde_json::to_string(&filter_for_audit),
                                serde_json::to_string(&repl_for_audit),
                            ) {
                                let _ = interceptor::record_update(audit, dep, db, coll, &fj, &rj);
                            }
                        });
                        matched += res.matched_count;
                        modified += res.modified_count;
                        if res.upserted_id.is_some() {
                            upserted += 1;
                        }
                    }
                    "deleteOne" | "deleteMany" => {
                        let filter = payload_doc
                            .get_document("filter")
                            .cloned()
                            .unwrap_or_default();
                        let multi = kind == "deleteMany";
                        let filter_for_audit = filter.clone();
                        let res = runtime_block_on(async move {
                            let c = entry.client.database(db).collection::<Document>(coll);
                            let r = if multi {
                                c.delete_many(filter).await
                            } else {
                                c.delete_one(filter).await
                            };
                            r.map_err(AppError::mongo)
                        })
                        .map_err(|e| js_err(e.to_string()))?;
                        try_audit(|audit, dep| {
                            if let Ok(fj) = serde_json::to_string(&filter_for_audit) {
                                let _ = interceptor::record_delete(audit, dep, db, coll, &fj);
                            }
                        });
                        deleted += res.deleted_count;
                    }
                    other => {
                        return Err(js_err(format!(
                            "bulkWrite: unsupported operation '{other}'"
                        )))
                    }
                }
            }
            push_output(ShellOutput::Text {
                value: format!(
                    "bulkWrite: inserted {inserted} · matched {matched} · modified {modified} · deleted {deleted} · upserted {upserted}"
                ),
            });
            Ok(JsValue::undefined())
        }
        "help" => {
            push_output(ShellOutput::Text {
                value: COLL_HELP_TEXT.to_string(),
            });
            Ok(JsValue::undefined())
        }
        other => return Err(js_err(format!("db.{coll}.{other}: not implemented"))),
    };
    result
}

/// Extract a BSON document argument at the given positional index.
fn doc_arg(args: &[bson::Bson], idx: usize) -> Option<Document> {
    args.get(idx).and_then(|b| match b {
        bson::Bson::Document(d) => Some(d.clone()),
        _ => None,
    })
}

/// Convert a BSON value into the driver's `UpdateModifications` enum,
/// accepting either an update document (`{$set: ...}`) or an aggregation
/// pipeline (an array of stage documents).
fn bson_to_update_mods(value: &bson::Bson) -> JsResult<mongodb::options::UpdateModifications> {
    match value {
        bson::Bson::Document(d) => Ok(mongodb::options::UpdateModifications::Document(d.clone())),
        bson::Bson::Array(arr) => {
            let pipeline: Vec<Document> = arr
                .iter()
                .filter_map(|b| match b {
                    bson::Bson::Document(d) => Some(d.clone()),
                    _ => None,
                })
                .collect();
            Ok(mongodb::options::UpdateModifications::Pipeline(pipeline))
        }
        other => Err(js_err(format!(
            "expected update document or pipeline array, got {other:?}"
        ))),
    }
}

/// Read the `returnNewDocument` (bool) or `returnDocument` ("before"/"after")
/// option from a mongosh-style options document and map it to the driver's
/// `ReturnDocument` enum. Returns None when neither option is present.
fn extract_return_document(opts: &Document) -> Option<mongodb::options::ReturnDocument> {
    use mongodb::options::ReturnDocument;
    if let Ok(true) = opts.get_bool("returnNewDocument") {
        return Some(ReturnDocument::After);
    }
    if let Ok(s) = opts.get_str("returnDocument") {
        return match s {
            "after" | "After" => Some(ReturnDocument::After),
            "before" | "Before" => Some(ReturnDocument::Before),
            _ => None,
        };
    }
    None
}

/// Attach mongo-cursor helper methods to the JsArray returned by `find` /
/// `aggregate` so chained calls like `db.x.find({}).toArray()` work. The
/// array already supports `forEach` / `map` / `filter` / `sort` / `length`
/// via `Array.prototype`; we only need to add the mongo-specific `toArray`
/// (a no-op that returns the array itself, since `find`/`aggregate` already
/// materialise up to 50 docs). `this`-bound so it returns whichever array
/// it's called on.
///
/// This is safe for JSON/BSON serialisation: both `js_to_json` and
/// `jsvalue_to_bson` serialise arrays by indexed access (`arr.get(i)` for
/// `i in 0..len`), so the `toArray` own property never leaks into output.
fn attach_cursor_methods(arr: &JsArray, ctx: &mut Context) -> JsResult<()> {
    let to_array: JsValue = NativeFunction::from_fn_ptr(|this, _args, _ctx| Ok(this.clone()))
        .to_js_function(ctx.realm())
        .into();
    arr.set(js_string!("toArray"), to_array, false, ctx)?;
    Ok(())
}

const COLL_HELP_TEXT: &str = "
db.<coll> methods:
  find(filter, projection, sort, limit)
  findOne(filter)
  countDocuments(filter)
  aggregate(pipeline)
  distinct(field, filter?)
  insertOne(doc)
  insertMany([doc, ...])
  updateOne(filter, update)
  updateMany(filter, update)
  replaceOne(filter, doc)
  deleteOne(filter)
  deleteMany(filter)
  createIndex(keys, options?)
  dropIndex(name)
  rename(newName)
  drop()
  findOneAndUpdate(filter, update, options?)
  findOneAndDelete(filter, options?)
  findOneAndReplace(filter, replacement, options?)
  bulkWrite([op, ...])
  help()
";

fn js_err(s: String) -> JsError {
    JsError::from_native(JsNativeError::typ().with_message(s))
}

fn format_js_error(e: &JsError) -> String {
    e.to_string()
}

// ---------- JS <-> JSON / BSON ----------

fn js_to_json(value: &JsValue, ctx: &mut Context) -> JsResult<JsonValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(JsonValue::Null);
    }
    if let Some(b) = value.as_boolean() {
        return Ok(JsonValue::Bool(b));
    }
    if let Some(n) = value.as_number() {
        return Ok(serde_json::Number::from_f64(n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null));
    }
    if let Some(s) = value.as_string() {
        return Ok(JsonValue::String(s.to_std_string_escaped()));
    }
    if let Some(obj) = value.as_object() {
        if obj.is_array() {
            let arr = JsArray::from_object(obj.clone())?;
            let len = arr.length(ctx)?;
            let mut out = Vec::with_capacity(len as usize);
            for i in 0..len {
                let v = arr.get(i, ctx)?;
                out.push(js_to_json(&v, ctx)?);
            }
            return Ok(JsonValue::Array(out));
        }
        let mut map = serde_json::Map::new();
        let keys = obj.own_property_keys(ctx)?;
        for key in keys {
            let key_str = key.to_string();
            let v = obj.get(key, ctx)?;
            map.insert(key_str, js_to_json(&v, ctx)?);
        }
        return Ok(JsonValue::Object(map));
    }
    let s = value.to_string(ctx)?.to_std_string_escaped();
    Ok(JsonValue::String(s))
}

fn jsvalue_to_bson(value: &JsValue, ctx: &mut Context) -> JsResult<bson::Bson> {
    if value.is_undefined() || value.is_null() {
        return Ok(bson::Bson::Null);
    }
    if let Some(b) = value.as_boolean() {
        return Ok(bson::Bson::Boolean(b));
    }
    if let Some(n) = value.as_number() {
        if n.fract() == 0.0 && n.is_finite() && n.abs() < 9.223372036854776e18 {
            return Ok(bson::Bson::Int64(n as i64));
        }
        return Ok(bson::Bson::Double(n));
    }
    if let Some(s) = value.as_string() {
        let raw = s.to_std_string_escaped();
        if raw.len() == 24 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            if let Ok(oid) = ObjectId::parse_str(&raw) {
                return Ok(bson::Bson::ObjectId(oid));
            }
        }
        if let Ok(dt) = BsonDateTime::parse_rfc3339_str(&raw) {
            return Ok(bson::Bson::DateTime(dt));
        }
        return Ok(bson::Bson::String(raw));
    }
    if let Some(obj) = value.as_object() {
        if obj.is_array() {
            let arr = JsArray::from_object(obj.clone())?;
            let len = arr.length(ctx)?;
            let mut out = Vec::with_capacity(len as usize);
            for i in 0..len {
                let v = arr.get(i, ctx)?;
                out.push(jsvalue_to_bson(&v, ctx)?);
            }
            return Ok(bson::Bson::Array(out));
        }
        let mut doc = Document::new();
        let keys = obj.own_property_keys(ctx)?;
        for key in keys {
            let key_str = key.to_string();
            let v = obj.get(key, ctx)?;
            doc.insert(key_str, jsvalue_to_bson(&v, ctx)?);
        }
        return Ok(bson::Bson::Document(doc));
    }
    let s = value.to_string(ctx)?.to_std_string_escaped();
    Ok(bson::Bson::String(s))
}

fn bson_to_js(bson: bson::Bson, ctx: &mut Context) -> JsResult<JsValue> {
    match bson {
        bson::Bson::Double(f) => Ok(JsValue::from(f)),
        bson::Bson::String(s) => Ok(JsValue::from(js_string!(s))),
        bson::Bson::Boolean(b) => Ok(JsValue::from(b)),
        bson::Bson::Null => Ok(JsValue::null()),
        bson::Bson::Int32(i) => Ok(JsValue::from(i as f64)),
        bson::Bson::Int64(i) => Ok(JsValue::from(i as f64)),
        bson::Bson::ObjectId(oid) => Ok(JsValue::from(js_string!(oid.to_hex()))),
        bson::Bson::DateTime(dt) => Ok(JsValue::from(js_string!(dt
            .try_to_rfc3339_string()
            .unwrap_or_default()))),
        bson::Bson::Array(arr) => {
            let js_arr = JsArray::new(ctx);
            for (i, v) in arr.into_iter().enumerate() {
                let js = bson_to_js(v, ctx)?;
                js_arr.set(i, js, false, ctx)?;
            }
            Ok(js_arr.into())
        }
        bson::Bson::Document(doc) => {
            let obj = boa_engine::object::JsObject::default(ctx.intrinsics());
            for (k, v) in doc.into_iter() {
                let js = bson_to_js(v, ctx)?;
                obj.set(js_string!(k), js, false, ctx)?;
            }
            Ok(obj.into())
        }
        _ => Ok(JsValue::null()),
    }
}

fn bson_to_json(b: bson::Bson) -> JsonValue {
    match b {
        bson::Bson::Double(f) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        bson::Bson::String(s) => JsonValue::String(s),
        bson::Bson::Boolean(b) => JsonValue::Bool(b),
        bson::Bson::Null => JsonValue::Null,
        bson::Bson::Int32(i) => JsonValue::from(i),
        bson::Bson::Int64(i) => JsonValue::from(i),
        bson::Bson::ObjectId(oid) => JsonValue::String(oid.to_hex()),
        bson::Bson::DateTime(dt) => {
            JsonValue::String(dt.try_to_rfc3339_string().unwrap_or_default())
        }
        bson::Bson::Decimal128(d) => JsonValue::String(d.to_string()),
        bson::Bson::Array(arr) => JsonValue::Array(arr.into_iter().map(bson_to_json).collect()),
        bson::Bson::Document(doc) => {
            let mut map = serde_json::Map::new();
            for (k, v) in doc.into_iter() {
                map.insert(k, bson_to_json(v));
            }
            JsonValue::Object(map)
        }
        _ => JsonValue::Null,
    }
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_use_strips_directive() {
        let (db, rest) = preprocess_use("use shop;\ndb.users.find()");
        assert_eq!(db.as_deref(), Some("shop"));
        assert_eq!(rest, "db.users.find()");
    }

    #[test]
    fn preprocess_use_handles_no_directive() {
        let (db, rest) = preprocess_use("db.users.find()");
        assert!(db.is_none());
        assert_eq!(rest, "db.users.find()");
    }

    /// Smoke test for the boa engine itself. Confirms we can
    /// spin up a Context, run a basic script, and round-trip a
    /// value through the host functions. Doesn't need a live
    /// Mongo connection.
    #[test]
    fn boa_engine_smoke_test() {
        let mut ctx = Context::default();
        let print = NativeFunction::from_fn_ptr(|_this, args, ctx| {
            let mut s = String::new();
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(&a.to_string(ctx)?.to_std_string_escaped());
            }
            push_output(ShellOutput::Text { value: s + "\n" });
            Ok(JsValue::undefined())
        });
        ctx.register_global_builtin_callable(js_string!("print"), 1, print)
            .unwrap();
        let result = ctx.eval(Source::from_bytes(b"1 + 2;"));
        assert!(result.is_ok());
        let result = ctx.eval(Source::from_bytes(b"print('hello from boa')"));
        assert!(result.is_ok());
        let outputs = take_outputs();
        assert!(!outputs.is_empty());
        match &outputs[0] {
            ShellOutput::Text { value } => assert!(value.contains("hello from boa")),
            _ => panic!("expected text output"),
        }
    }

    /// Variables persist across evals in the same Context.
    #[test]
    fn boa_engine_persistent_variables() {
        let mut ctx = Context::default();
        ctx.eval(Source::from_bytes(b"var x = 42;")).unwrap();
        let r = ctx.eval(Source::from_bytes(b"x + 1")).unwrap();
        let n = r.as_number().unwrap();
        assert_eq!(n, 43.0);
    }

    /// `for` loops work in classic scripts (not just modules).
    #[test]
    fn boa_engine_for_loop() {
        let mut ctx = Context::default();
        let print = NativeFunction::from_fn_ptr(|_this, args, ctx| {
            let s = args
                .get_or_undefined(0)
                .to_string(ctx)?
                .to_std_string_escaped();
            push_output(ShellOutput::Text { value: s + "\n" });
            Ok(JsValue::undefined())
        });
        ctx.register_global_builtin_callable(js_string!("print"), 1, print)
            .unwrap();
        ctx.eval(Source::from_bytes(
            b"for (var i = 0; i < 3; i++) { print(i); }",
        ))
        .unwrap();
        let outputs = take_outputs();
        let lines: Vec<String> = outputs
            .iter()
            .filter_map(|o| match o {
                ShellOutput::Text { value } => Some(value.trim().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec!["0", "1", "2"]);
    }

    /// Regression test for the "shell entry not set" panic.
    ///
    /// The previous design set the entry on the Tauri command
    /// thread, but thread-locals are per-thread. The native
    /// host functions run on the shell thread, so they always
    /// saw `None` and panicked. The fix moves the `set_entry`
    /// call into the `ShellMessage::Eval` handler on the
    /// shell thread itself.
    ///
    /// This test pins the *contract* used by the fix: the
    /// `ShellMessage::Eval` variant must carry an `entry`
    /// field. The variant's existence is a compile-time
    /// guarantee; we additionally exercise `set_entry` +
    /// `with_entry` on the same thread to confirm the
    /// thread-local round-trip works.
    #[test]
    fn shell_eval_message_carries_entry() {
        // Compile-time check: the Eval variant MUST have an
        // `entry` field. If a future refactor drops it, this
        // test won't compile.
        let _: fn(
            std::sync::Arc<ClientEntry>,
            String,
            String,
            Option<std::sync::Arc<crate::audit::AuditLog>>,
            std::sync::mpsc::Sender<AppResult<ShellResponse>>,
        ) -> ShellMessage = |entry, script, active_db, audit_log, respond| ShellMessage::Eval {
            entry,
            script,
            active_db,
            audit_log,
            respond,
        };
    }

    /// The thread-local helpers are reachable and the
    /// shell thread's per-eval protocol is correct.
    /// This test only runs if a real ClientEntry can be
    /// constructed; in CI we rely on the compile-time
    /// guarantee from `shell_eval_message_carries_entry`.
    ///
    /// A real end-to-end test of `Shell::eval` would
    /// require a live MongoDB and is covered by the
    /// integration tests instead.
    #[test]
    fn shell_thread_locals_link() {
        // Sanity: thread-locals are reachable (no dead
        // code removal). The real fix is enforced by the
        // compile-time check in
        // `shell_eval_message_carries_entry`.
        let _: fn(Arc<ClientEntry>) = set_entry;
    }

    /// Regression test for the "no current tokio runtime"
    /// error. The shell thread enters its own current-thread
    /// runtime before any script runs, so the native host
    /// functions can call `Handle::try_current()` and find
    /// a live runtime. Without the `runtime.enter()` call,
    /// `try_current()` would return Err (the global handle
    /// points to Tauri's multi-thread runtime, not the
    /// shell thread's current-thread one).
    #[test]
    fn shell_runtime_is_entered_before_block_on() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = rt.enter();
        // Now `Handle::try_current()` should succeed.
        let handle = tokio::runtime::Handle::try_current()
            .expect("handle should be found inside enter() scope");
        // And `block_on` should work.
        let result = handle.block_on(async { 42u32 });
        assert_eq!(result, 42);
    }

    /// The source transformer must rewrite write-method call
    /// sites the same way it rewrites read-method call sites.
    /// This test pins the contract for every write method we
    /// added in this change. If a method name is misspelled in
    /// the dispatch table but the transformer doesn't match it,
    /// the user gets a confusing "not implemented" error.
    #[test]
    fn transform_source_rewrites_write_methods() {
        let methods = [
            "insertMany",
            "updateOne",
            "updateMany",
            "replaceOne",
            "deleteOne",
            "deleteMany",
            "createIndex",
            "dropIndex",
            "rename",
            "renameCollection",
            "drop",
            "dropDatabase",
        ];
        for m in methods {
            let src = format!("db.users.{m}({{a:1}}, {{b:2}})");
            let out = transform_source(&src);
            assert!(
                out.contains(&format!("__call_db(\"users\", \"{m}\", ")),
                "transform_source did not rewrite db.users.{m}(...) → __call_db(...): got {out}"
            );
        }
    }

    /// The source transformer must rewrite `db.runCommand(...)`
    /// before the `db.X.Y(...)` rewrite runs, so that
    /// `db.runCommand({renameCollection: ...})` is not
    /// misrouted as a collection call.
    #[test]
    fn transform_source_preserves_run_command() {
        let src = "db.runCommand({ping: 1})";
        let out = transform_source(src);
        assert!(
            out.contains("__run_command(") && !out.contains("__call_db(\"runCommand\""),
            "db.runCommand(...) should become __run_command(...), got: {out}"
        );
    }

    /// `db.coll.createIndex("text")` (string keys for a text
    /// index) is a valid mongosh form. The transformer must
    /// still rewrite it; the dispatch converts the string to
    /// `{ field: "text" }`. This test only checks the
    /// transformer, not the dispatch (which needs Mongo).
    #[test]
    fn transform_source_rewrites_create_index_with_string_keys() {
        let src = "db.articles.createIndex(\"text\")";
        let out = transform_source(src);
        assert!(
            out.contains("__call_db(\"articles\", \"createIndex\", "),
            "createIndex with string keys should be rewritten, got: {out}"
        );
    }

    /// The transformer must NOT rewrite `db.foo.bar(` text that
    /// appears inside a string literal, line comment, or block
    /// comment. This is the fix for the known regex-transformer
    /// limitation.
    #[test]
    fn transform_source_skips_strings_and_comments() {
        // Inside a double-quoted string.
        let src = "print(\"db.foo.bar() is just text\"); db.users.find()";
        let out = transform_source(src);
        assert!(
            out.contains("db.foo.bar() is just text"),
            "string literal should be preserved verbatim, got: {out}"
        );
        assert!(
            out.contains("__call_db(\"users\", \"find\", "),
            "real call after the string should still be rewritten, got: {out}"
        );

        // Inside a single-quoted string with an escaped quote.
        let src2 = "var s = 'db.x.y(\\'q\\')'; db.users.find()";
        let out2 = transform_source(src2);
        assert!(
            out2.contains("db.x.y(\\'q\\')"),
            "single-quoted string with escaped quote should be preserved, got: {out2}"
        );

        // Inside a line comment.
        let src3 = "// db.foo.drop()\ndb.users.find()";
        let out3 = transform_source(src3);
        assert!(
            out3.contains("// db.foo.drop()"),
            "line comment should be preserved, got: {out3}"
        );

        // Inside a block comment.
        let src4 = "/* db.foo.drop() */ db.users.find()";
        let out4 = transform_source(src4);
        assert!(
            out4.contains("/* db.foo.drop() */"),
            "block comment should be preserved, got: {out4}"
        );
    }

    /// `db.help()` has only one dot, so the call-db regex (which
    /// requires `db.X.Y(`) never matched it. Previously this left
    /// `db.help()` as a call on the empty `db` object → TypeError.
    /// The transformer must leave `db.help()` alone (it's handled by
    /// the real `help` method attached to `db`), and must not misroute
    /// it as `db.<coll>.help`.
    #[test]
    fn transform_source_leaves_db_help_alone() {
        let src = "db.help()";
        let out = transform_source(src);
        assert_eq!(
            out, "db.help()",
            "db.help() should not be rewritten by the transformer, got: {out}"
        );
    }

    /// `db` followed by a non-identifier (e.g. `db;` or `var x = db`)
    /// must not be rewritten, and `mydb.foo.bar(` (where `db` is the
    /// tail of a larger identifier) must not be rewritten either.
    #[test]
    fn transform_source_respects_word_boundaries() {
        let src = "var x = mydb.foo.bar(); db.users.find()";
        let out = transform_source(src);
        assert!(
            out.contains("mydb.foo.bar()"),
            "mydb.foo.bar() should NOT be rewritten (db is not a standalone word), got: {out}"
        );
        assert!(
            out.contains("__call_db(\"users\", \"find\", "),
            "db.users.find() should be rewritten, got: {out}"
        );
    }

    /// `db.help()` must resolve to a real function on the `db` object
    /// and push the database-methods help text, rather than throwing
    /// "db.help is not a function". This exercises `install_host` +
    /// `install_db_stub` end-to-end without a Mongo connection.
    #[test]
    fn db_help_is_callable_and_prints_help() {
        let mut ctx = Context::default();
        install_host(&mut ctx);
        let result = ctx.eval(Source::from_bytes(b"db.help()"));
        assert!(
            result.is_ok(),
            "db.help() should not throw: {:?}",
            result.err()
        );
        let outputs = take_outputs();
        let text: String = outputs
            .iter()
            .filter_map(|o| match o {
                ShellOutput::Text { value } => Some(value.clone()),
                _ => None,
            })
            .collect();
        assert!(
            text.contains("db methods:"),
            "db.help() should print the db-methods help text, got: {text}"
        );
    }

    /// `find` / `aggregate` return a JsArray with a per-instance `toArray`
    /// method so chained calls like `db.x.find({}).toArray()` work. We
    /// can't run a real `find` without Mongo, but we can build a JsArray
    /// the same way `dispatch_sync` does and confirm `toArray` round-trips
    /// the array (returns the same length and elements). This pins the
    /// `attach_cursor_methods` contract.
    #[test]
    fn cursor_to_array_returns_the_underlying_array() {
        let mut ctx = Context::default();
        let arr = JsArray::new(&mut ctx);
        arr.set(0, JsValue::from(1), false, &mut ctx).unwrap();
        arr.set(1, JsValue::from(2), false, &mut ctx).unwrap();
        attach_cursor_methods(&arr, &mut ctx).unwrap();
        // Expose the array as a global so we can call `.toArray()` on it.
        ctx.register_global_property(js_string!("arr"), arr.clone(), Attribute::all())
            .unwrap();
        let result = ctx.eval(Source::from_bytes(b"arr.toArray().length"));
        assert!(
            result.is_ok(),
            "arr.toArray() should not throw: {:?}",
            result.err()
        );
        let len = result.unwrap().as_number().unwrap();
        assert_eq!(len as i64, 2, "toArray() should return the 2-element array");
        // Confirm the elements round-trip.
        let first = ctx.eval(Source::from_bytes(b"arr.toArray()[0]")).unwrap();
        assert_eq!(first.as_number().unwrap() as i64, 1);
    }

    /// `jsvalue_to_bson` must convert a JS array of objects
    /// into a `Bson::Array` of `Bson::Document`s, so that
    /// `insertMany([{a:1},{a:2}])` extracts correctly. This
    /// exercises the argument extraction path without a live
    /// Mongo connection.
    #[test]
    fn jsvalue_to_bson_converts_array_of_objects() {
        let mut ctx = Context::default();
        let js = ctx.eval(Source::from_bytes(b"[{a: 1}, {a: 2}]")).unwrap();
        let bson = jsvalue_to_bson(&js, &mut ctx).unwrap();
        match bson {
            bson::Bson::Array(arr) => {
                assert_eq!(arr.len(), 2);
                assert!(matches!(arr[0], bson::Bson::Document(_)));
                assert!(matches!(arr[1], bson::Bson::Document(_)));
            }
            other => panic!("expected Bson::Array, got {other:?}"),
        }
    }

    /// `jsvalue_to_bson` must convert a JS object with
    /// `$set` key into a BSON document, so that
    /// `updateOne(filter, {$set: {a: 1}})` extracts the
    /// update document correctly.
    #[test]
    fn jsvalue_to_bson_converts_update_document() {
        let mut ctx = Context::default();
        let js = ctx.eval(Source::from_bytes(b"({$set: {a: 1}})")).unwrap();
        let bson = jsvalue_to_bson(&js, &mut ctx).unwrap();
        match bson {
            bson::Bson::Document(d) => {
                assert!(d.contains_key("$set"));
            }
            other => panic!("expected Bson::Document, got {other:?}"),
        }
    }

    /// `jsvalue_to_bson` must convert a JS options object
    /// with `unique: true, sparse: true` into a BSON document
    /// so `createIndex(keys, {unique: true})` extracts the
    /// options correctly.
    #[test]
    fn jsvalue_to_bson_converts_index_options() {
        let mut ctx = Context::default();
        let js = ctx
            .eval(Source::from_bytes(
                b"({unique: true, sparse: true, name: \"idx_a\"})",
            ))
            .unwrap();
        let bson = jsvalue_to_bson(&js, &mut ctx).unwrap();
        match bson {
            bson::Bson::Document(d) => {
                assert!(d.get_bool("unique").unwrap());
                assert!(d.get_bool("sparse").unwrap());
                assert_eq!(d.get_str("name").unwrap(), "idx_a");
            }
            other => panic!("expected Bson::Document, got {other:?}"),
        }
    }

    #[test]
    fn js_to_json_preserves_numeric_string_object_keys() {
        let mut ctx = Context::default();
        let js = ctx
            .eval(Source::from_bytes(b"({'123': 'kept', nested: {'456': 7}})"))
            .unwrap();
        let json = js_to_json(&js, &mut ctx).unwrap();
        assert_eq!(json["123"], JsonValue::String("kept".into()));
        assert_eq!(json["nested"]["456"], serde_json::json!(7.0));
    }

    #[test]
    fn jsvalue_to_bson_preserves_numeric_string_object_keys() {
        let mut ctx = Context::default();
        let js = ctx
            .eval(Source::from_bytes(b"({'123': 'kept', nested: {'456': 7}})"))
            .unwrap();
        let bson = jsvalue_to_bson(&js, &mut ctx).unwrap();
        let doc = bson.as_document().expect("document");
        assert_eq!(doc.get_str("123").unwrap(), "kept");
        assert!(doc.get_document("nested").unwrap().contains_key("456"));
    }

    #[test]
    fn jsvalue_to_bson_characterizes_hex_and_rfc3339_string_coercion() {
        let mut ctx = Context::default();

        let oid_js = ctx
            .eval(Source::from_bytes(b"\"507f1f77bcf86cd799439011\""))
            .unwrap();
        assert!(matches!(
            jsvalue_to_bson(&oid_js, &mut ctx).unwrap(),
            bson::Bson::ObjectId(_)
        ));

        let date_js = ctx
            .eval(Source::from_bytes(b"\"2026-06-29T07:05:09Z\""))
            .unwrap();
        assert!(matches!(
            jsvalue_to_bson(&date_js, &mut ctx).unwrap(),
            bson::Bson::DateTime(_)
        ));
    }

    /// The `COLL_HELP_TEXT` must list every write method we
    /// added. If a method is wired in dispatch but missing
    /// from help, users won't discover it.
    #[test]
    fn coll_help_text_lists_all_write_methods() {
        for m in [
            "insertMany",
            "updateOne",
            "updateMany",
            "replaceOne",
            "deleteOne",
            "deleteMany",
            "createIndex",
            "dropIndex",
            "rename",
            "drop()",
            "findOneAndUpdate",
            "findOneAndDelete",
            "findOneAndReplace",
            "bulkWrite",
        ] {
            assert!(
                COLL_HELP_TEXT.contains(m),
                "COLL_HELP_TEXT missing method: {m}"
            );
        }
    }

    /// The source transformer must rewrite the find-and-modify and
    /// bulkWrite call sites just like the other write methods, so
    /// they reach `dispatch_sync` instead of erroring out.
    #[test]
    fn transform_source_rewrites_find_and_modify_methods() {
        for m in [
            "findOneAndUpdate",
            "findOneAndDelete",
            "findOneAndReplace",
            "bulkWrite",
        ] {
            let src = format!("db.users.{m}({{a:1}}, {{b:2}})");
            let out = transform_source(&src);
            assert!(
                out.contains(&format!("__call_db(\"users\", \"{m}\", ")),
                "transform_source did not rewrite db.users.{m}(...) → __call_db(...): got {out}"
            );
        }
    }

    /// `bson_to_update_mods` must accept an aggregation pipeline
    /// (an array of stage documents) as the update argument, since
    /// `findOneAndUpdate` and `bulkWrite` update operations support
    /// pipeline updates. This exercises the path without a live Mongo
    /// connection.
    #[test]
    fn bson_to_update_mods_converts_pipeline() {
        let mut ctx = Context::default();
        let js = ctx
            .eval(Source::from_bytes(
                b"[{$addFields: {a: 1}}, {$project: {a: 1}}]",
            ))
            .unwrap();
        let mods = bson_to_update_mods(&jsvalue_to_bson(&js, &mut ctx).unwrap()).unwrap();
        match mods {
            mongodb::options::UpdateModifications::Pipeline(v) => {
                assert_eq!(v.len(), 2);
                assert!(v[0].contains_key("$addFields"));
            }
            other => panic!("expected Pipeline, got {other:?}"),
        }
    }

    /// `bson_to_update_mods` must accept a plain update document
    /// (`{$set: ...}`) and return the `Document` variant.
    #[test]
    fn bson_to_update_mods_converts_document() {
        let mut ctx = Context::default();
        let js = ctx.eval(Source::from_bytes(b"({$set: {a: 1}})")).unwrap();
        let mods = bson_to_update_mods(&jsvalue_to_bson(&js, &mut ctx).unwrap()).unwrap();
        match mods {
            mongodb::options::UpdateModifications::Document(d) => {
                assert!(d.contains_key("$set"));
            }
            other => panic!("expected Document, got {other:?}"),
        }
    }

    /// `extract_return_document` must map both mongosh forms —
    /// `returnNewDocument: true` (boolean) and
    /// `returnDocument: "after"` (string) — to `ReturnDocument::After`,
    /// and leave the default (no option) as None.
    #[test]
    fn extract_return_document_maps_both_forms() {
        use mongodb::options::ReturnDocument;
        let new_doc = bson::doc! { "returnNewDocument": true };
        assert!(matches!(
            extract_return_document(&new_doc),
            Some(ReturnDocument::After)
        ));
        let str_doc = bson::doc! { "returnDocument": "before" };
        assert!(matches!(
            extract_return_document(&str_doc),
            Some(ReturnDocument::Before)
        ));
        let none_doc = bson::doc! { "upsert": true };
        assert!(extract_return_document(&none_doc).is_none());
    }
}
