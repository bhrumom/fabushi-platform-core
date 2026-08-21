//! Stable C/JSON compatibility ABI for native Mahayana hosts.
//!
//! Runtime construction and behavior live in `mahayana-host`. This crate owns
//! only C pointers, JSON conversion, process-local numeric handles, and the
//! stable native ABI symbols used by application host adapters.

use mahayana_core::ApprovalDecision;
use mahayana_core::ApprovalId;
use mahayana_core::OperationId;
use mahayana_core::RuntimeCommand;
use mahayana_host::HostCreateConfig;
use mahayana_host::MahayanaHost;
use mahayana_product::MahayanaProductClient;
use once_cell::sync::Lazy;
use serde_json::Value;
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CStr;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);
static RUNTIMES: Lazy<Mutex<HashMap<u64, Arc<MahayanaHost>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Creates a long-lived direct Rust host and stores it behind a stable numeric
/// handle. A null config pointer uses safe defaults.
///
/// Returns zero on error; call [`mahayana_runtime_last_error`] for details.
///
/// # Safety
/// A non-null `config_json` must point to valid NUL-terminated UTF-8 for the
/// duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_create(config_json: *const c_char) -> u64 {
    clear_last_error();
    let result = (|| {
        let create_config: HostCreateConfig = if config_json.is_null() {
            HostCreateConfig::default()
        } else {
            let source = unsafe { CStr::from_ptr(config_json) }
                .to_str()
                .map_err(|error| format!("runtime config must be UTF-8: {error}"))?;
            serde_json::from_str(source)
                .map_err(|error| format!("runtime config must be valid JSON: {error}"))?
        };
        let host = MahayanaHost::create(create_config).map_err(|error| error.to_string())?;
        let runtime_id = NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed);
        RUNTIMES
            .lock()
            .map_err(|_| "runtime registry mutex poisoned".to_string())?
            .insert(runtime_id, Arc::new(host));
        Ok::<_, String>(runtime_id)
    })();
    match result {
        Ok(runtime_id) => runtime_id,
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

/// Executes one typed Runtime command and returns an owned JSON envelope.
///
/// # Safety
/// `command_json` must be a valid NUL-terminated UTF-8 string. Release the
/// returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_execute(
    runtime_id: u64,
    command_json: *const c_char,
) -> *mut c_char {
    ffi_response(|| {
        let command: RuntimeCommand = unsafe { parse_json(command_json, "command") }?;
        runtime(runtime_id)?
            .execute(command)
            .map_err(|error| error.to_string())
    })
}

/// Receives the next Runtime event, waiting for at most `timeout_ms`.
///
/// # Safety
/// Release the returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_receive(runtime_id: u64, timeout_ms: u64) -> *mut c_char {
    ffi_response(|| {
        runtime(runtime_id)?
            .receive(Duration::from_millis(timeout_ms))
            .map_err(|error| error.to_string())
    })
}

/// Interrupts an operation using `{ "operationId": "..." }`.
///
/// # Safety
/// `operation_json` must be a valid NUL-terminated UTF-8 string. Release the
/// returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_interrupt(
    runtime_id: u64,
    operation_json: *const c_char,
) -> *mut c_char {
    ffi_response(|| {
        let payload: Value = unsafe { parse_json(operation_json, "operation") }?;
        let operation_id = OperationId::new(required_string(&payload, "operationId")?)
            .map_err(|error| error.to_string())?;
        runtime(runtime_id)?
            .interrupt(operation_id)
            .map_err(|error| error.to_string())
    })
}

/// Resolves an approval using `{ "approvalId", "decision", "payload"? }`.
///
/// # Safety
/// `approval_json` must point to valid NUL-terminated UTF-8. Release the
/// returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_resolve_approval(
    runtime_id: u64,
    approval_json: *const c_char,
) -> *mut c_char {
    ffi_response(|| {
        let payload: Value = unsafe { parse_json(approval_json, "approval") }?;
        let approval_id = ApprovalId::new(required_string(&payload, "approvalId")?)
            .map_err(|error| error.to_string())?;
        let decision: ApprovalDecision = serde_json::from_value(
            payload
                .get("decision")
                .cloned()
                .ok_or_else(|| "approval decision is required".to_string())?,
        )
        .map_err(|error| format!("approval decision is invalid: {error}"))?;
        runtime(runtime_id)?
            .resolve_approval(
                approval_id,
                decision,
                payload.get("payload").cloned().unwrap_or(Value::Null),
            )
            .map_err(|error| error.to_string())
    })
}

/// Closes and removes a Runtime handle.
///
/// # Safety
/// Release the returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_close(runtime_id: u64) -> *mut c_char {
    ffi_response(|| {
        let removed = RUNTIMES
            .lock()
            .map_err(|_| "runtime registry mutex poisoned".to_string())?
            .remove(&runtime_id)
            .is_some();
        if !removed {
            return Err(format!("runtime was not found: {runtime_id}"));
        }
        Ok(json!({"runtimeId": runtime_id, "closed": true}))
    })
}

/// Returns the most recent creation error for the calling thread.
///
/// # Safety
/// Release the returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_last_error() -> *mut c_char {
    let error = LAST_ERROR.with(|slot| slot.borrow().clone());
    into_c_string(json!({"ok": error.is_none(), "message": error}))
}

/// Executes first-party account, contact-management, or direct-message
/// commands that are outside the long-lived conversation Runtime.
///
/// # Safety
/// `request_json` must point to valid NUL-terminated UTF-8 JSON. Release the
/// returned pointer with [`mahayana_runtime_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_product_execute(request_json: *const c_char) -> *mut c_char {
    ffi_response(|| {
        let request: Value = unsafe { parse_json(request_json, "product request") }?;
        let request_type = required_string(&request, "@type")?;
        MahayanaProductClient::default()
            .execute(request_type, &request)
            .map_err(|error| error.to_string())
    })
}

/// Linker anchor for native builds that resolve Runtime and Telegram
/// symbols from one shared artifact.
#[unsafe(no_mangle)]
pub extern "C" fn mahayana_runtime_force_link() -> u32 {
    let runtime_symbols = [
        mahayana_runtime_create as *const () as usize,
        mahayana_runtime_execute as *const () as usize,
        mahayana_runtime_receive as *const () as usize,
        mahayana_runtime_interrupt as *const () as usize,
        mahayana_runtime_resolve_approval as *const () as usize,
        mahayana_runtime_close as *const () as usize,
        mahayana_product_execute as *const () as usize,
        mahayana_runtime_free_string as *const () as usize,
    ];
    let telegram_symbols = [
        fabushi_telegram_runtime::fabushi_telegram_create_client as *const () as usize,
        fabushi_telegram_runtime::fabushi_telegram_create_persistent_client as *const () as usize,
        fabushi_telegram_runtime::fabushi_telegram_execute as *const () as usize,
        fabushi_telegram_runtime::fabushi_telegram_close_client as *const () as usize,
        fabushi_telegram_runtime::fabushi_telegram_free_string as *const () as usize,
    ];
    std::hint::black_box((runtime_symbols, telegram_symbols));
    1
}

/// Releases strings returned by this ABI.
///
/// # Safety
/// `pointer` must be null or a pointer returned by a Mahayana string-returning
/// ABI function, and it must be released exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_runtime_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(unsafe { CString::from_raw(pointer) });
    }
}

fn runtime(runtime_id: u64) -> Result<Arc<MahayanaHost>, String> {
    RUNTIMES
        .lock()
        .map_err(|_| "runtime registry mutex poisoned".to_string())?
        .get(&runtime_id)
        .cloned()
        .ok_or_else(|| format!("runtime was not found: {runtime_id}"))
}

unsafe fn parse_json<T>(pointer: *const c_char, name: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    if pointer.is_null() {
        return Err(format!("{name} JSON must not be null"));
    }
    let source = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map_err(|error| format!("{name} JSON must be UTF-8: {error}"))?;
    serde_json::from_str(source).map_err(|error| format!("{name} JSON is invalid: {error}"))
}

fn required_string<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

fn ffi_response<T>(operation: impl FnOnce() -> Result<T, String>) -> *mut c_char
where
    T: serde::Serialize,
{
    let response = match operation() {
        Ok(data) => json!({"ok": true, "data": data}),
        Err(message) => json!({
            "ok": false,
            "errorCode": "mahayana_runtime_error",
            "message": message,
        }),
    };
    into_c_string(response)
}

fn into_c_string(value: Value) -> *mut c_char {
    let encoded = serde_json::to_string(&value).unwrap_or_else(|_| {
        "{\"ok\":false,\"errorCode\":\"mahayana_serialization_error\"}".to_string()
    });
    CString::new(encoded)
        .expect("serialized JSON contains no NUL")
        .into_raw()
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

fn set_last_error(error: String) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(error));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(pointer: *mut c_char) -> Value {
        assert!(!pointer.is_null());
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .expect("UTF-8 response")
            .to_string();
        unsafe { mahayana_runtime_free_string(pointer) };
        serde_json::from_str(&text).expect("JSON response")
    }

    #[test]
    fn lifecycle_abi_creates_executes_receives_and_closes() {
        let runtime_id = unsafe { mahayana_runtime_create(std::ptr::null()) };
        assert_ne!(runtime_id, 0);
        let command = CString::new(r#"{"@type":"mahayana.runtime.status"}"#).unwrap();
        let status = take(unsafe { mahayana_runtime_execute(runtime_id, command.as_ptr()) });
        assert_eq!(status["ok"], true);
        assert_eq!(status["data"]["runtimeAbiVersion"], 1);
        assert_eq!(status["data"]["remoteAgentEnabled"], false);

        let ready = take(unsafe { mahayana_runtime_receive(runtime_id, 10) });
        assert_eq!(ready["ok"], true);
        assert_eq!(ready["data"]["@type"], "mahayana.runtime.ready");

        let closed = take(unsafe { mahayana_runtime_close(runtime_id) });
        assert_eq!(closed["data"]["closed"], true);
    }

    #[test]
    fn create_rejects_remote_agent_gateway() {
        let config = CString::new(r#"{"remoteAgentEnabled":true}"#).unwrap();
        let runtime_id = unsafe { mahayana_runtime_create(config.as_ptr()) };
        assert_eq!(runtime_id, 0);
        let error = take(unsafe { mahayana_runtime_last_error() });
        assert!(
            error["message"]
                .as_str()
                .expect("error message")
                .contains("remote Agent")
        );
    }

    #[test]
    fn unknown_handle_preserves_legacy_error_envelope() {
        let command = CString::new(r#"{"@type":"mahayana.runtime.status"}"#).unwrap();
        let response = take(unsafe { mahayana_runtime_execute(u64::MAX, command.as_ptr()) });
        assert_eq!(response["ok"], false);
        assert_eq!(response["errorCode"], "mahayana_runtime_error");
        assert!(response["message"].as_str().unwrap().contains("not found"));
    }
}
