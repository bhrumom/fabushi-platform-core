use mahayana_app_host::{AppHostFeatureMode, HostResponse, default_app_data_dir};
use mahayana_unified_app_host::{UnifiedAppHost, dispatch_json};
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;

/// Creates a native app-host handle.
///
/// # Safety
/// If `app_data_dir` is non-null, it must point to a valid NUL-terminated C string
/// for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create(
    app_data_dir: *const c_char,
) -> *mut UnifiedAppHost {
    let path = if app_data_dir.is_null() {
        default_app_data_dir()
    } else {
        PathBuf::from(
            unsafe { CStr::from_ptr(app_data_dir) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    match UnifiedAppHost::new(path) {
        Ok(host) => Box::into_raw(Box::new(host)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Creates a production native app-host with a stable storage passphrase supplied
/// by the platform Keychain/Keystore bridge. The passphrase is consumed in memory
/// and never written to the Rust app-data directory.
///
/// # Safety
/// Both pointers must reference valid NUL-terminated strings for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create_with_storage_passphrase(
    app_data_dir: *const c_char,
    storage_passphrase: *const c_char,
) -> *mut UnifiedAppHost {
    if storage_passphrase.is_null() {
        return std::ptr::null_mut();
    }
    let path = if app_data_dir.is_null() {
        default_app_data_dir()
    } else {
        PathBuf::from(
            unsafe { CStr::from_ptr(app_data_dir) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    let passphrase = unsafe { CStr::from_ptr(storage_passphrase) }
        .to_string_lossy()
        .into_owned();
    if passphrase.is_empty() {
        return std::ptr::null_mut();
    }
    match UnifiedAppHost::new_with_feature_mode_and_storage_passphrase(
        path,
        AppHostFeatureMode::Production,
        passphrase,
    ) {
        Ok(host) => Box::into_raw(Box::new(host)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Creates a native app-host handle backed by the deterministic FeatureHost test mode.
/// This is used only by explicit UI/instrumentation test harnesses; normal app
/// creation continues to use the production mode.
///
/// # Safety
/// `app_data_dir` must follow the same contract as `mahayana_app_host_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create_test(
    app_data_dir: *const c_char,
) -> *mut UnifiedAppHost {
    let path = if app_data_dir.is_null() {
        default_app_data_dir()
    } else {
        PathBuf::from(
            unsafe { CStr::from_ptr(app_data_dir) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    match UnifiedAppHost::new_with_feature_mode(path, AppHostFeatureMode::Test) {
        Ok(host) => Box::into_raw(Box::new(host)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Dispatches one JSON request through an existing native app-host handle.
///
/// # Safety
/// `host` must be a live pointer returned by `mahayana_app_host_create`, and
/// `request_json` must point to a valid NUL-terminated C string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_dispatch_with_handle(
    host: *mut UnifiedAppHost,
    request_json: *const c_char,
) -> *mut c_char {
    if host.is_null() || request_json.is_null() {
        return CString::new("{\"ok\":false,\"error\":\"null host or request\"}")
            .unwrap()
            .into_raw();
    }
    let input = unsafe { CStr::from_ptr(request_json) }.to_string_lossy();
    let output = dispatch_json(unsafe { &*host }, &input);
    CString::new(output)
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"invalid response\"}").unwrap())
        .into_raw()
}

/// Destroys a native app-host handle.
///
/// # Safety
/// `host` must be null or a live pointer returned by `mahayana_app_host_create`
/// that has not previously been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_destroy(host: *mut UnifiedAppHost) {
    if !host.is_null() {
        unsafe {
            drop(Box::from_raw(host));
        }
    }
}

/// Dispatches one JSON request using a temporary default app-host.
///
/// # Safety
/// `request_json` must point to a valid NUL-terminated C string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_dispatch(request_json: *const c_char) -> *mut c_char {
    if request_json.is_null() {
        return CString::new("{\"ok\":false,\"error\":\"null request\"}")
            .unwrap()
            .into_raw();
    }
    let input = unsafe { CStr::from_ptr(request_json) }.to_string_lossy();
    let output = match UnifiedAppHost::new(default_app_data_dir()) {
        Ok(host) => dispatch_json(&host, &input),
        Err(error) => serde_json::to_string(&HostResponse {
            id: None,
            ok: false,
            result: None,
            error: Some(error.to_string()),
        })
        .unwrap(),
    };
    CString::new(output)
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"invalid response\"}").unwrap())
        .into_raw()
}

/// Frees a response string returned by this FFI module.
///
/// # Safety
/// `pointer` must be null or a pointer returned by a Mahayana app-host dispatch
/// function that has not previously been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_free_string(pointer: *mut c_char) {
    if !pointer.is_null() {
        unsafe {
            drop(CString::from_raw(pointer));
        }
    }
}

#[cfg(target_os = "android")]
mod android_jni {
    use super::*;
    use jni::JNIEnv;
    use jni::objects::{JObject, JString};
    use jni::sys::{jlong, jstring};

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_ombhrum_fabushi_core_MahayanaHost_nativeCreate(
        mut env: JNIEnv,
        _object: JObject,
        app_data_dir: JString,
        storage_passphrase: JString,
    ) -> jlong {
        let path = match env.get_string(&app_data_dir) {
            Ok(value) => PathBuf::from(value.to_string_lossy().into_owned()),
            Err(_) => return 0,
        };
        let passphrase = match env.get_string(&storage_passphrase) {
            Ok(value) => value.to_string_lossy().into_owned(),
            Err(_) => return 0,
        };
        if passphrase.is_empty() {
            return 0;
        }
        match UnifiedAppHost::new_with_feature_mode_and_storage_passphrase(
            path,
            AppHostFeatureMode::Production,
            passphrase,
        ) {
            Ok(host) => Box::into_raw(Box::new(host)) as jlong,
            Err(_) => 0,
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_ombhrum_fabushi_core_MahayanaHost_nativeCreateTest(
        mut env: JNIEnv,
        _object: JObject,
        app_data_dir: JString,
    ) -> jlong {
        let path = match env.get_string(&app_data_dir) {
            Ok(value) => PathBuf::from(value.to_string_lossy().into_owned()),
            Err(_) => return 0,
        };
        match UnifiedAppHost::new_with_feature_mode(path, AppHostFeatureMode::Test) {
            Ok(host) => Box::into_raw(Box::new(host)) as jlong,
            Err(_) => 0,
        }
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_ombhrum_fabushi_core_MahayanaHost_nativeDispatch(
        mut env: JNIEnv,
        _object: JObject,
        handle: jlong,
        request_json: JString,
    ) -> jstring {
        if handle == 0 {
            return env
                .new_string("{\"ok\":false,\"error\":\"native host is not initialized\"}")
                .map(|value| value.into_raw())
                .unwrap_or(std::ptr::null_mut());
        }
        let input = match env.get_string(&request_json) {
            Ok(value) => value.to_string_lossy().into_owned(),
            Err(error) => format!("{{\"ok\":false,\"error\":\"invalid request: {error}\"}}"),
        };
        let host = unsafe { &*(handle as *mut UnifiedAppHost) };
        env.new_string(dispatch_json(host, &input))
            .map(|value| value.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_com_ombhrum_fabushi_core_MahayanaHost_nativeDestroy(
        _env: JNIEnv,
        _object: JObject,
        handle: jlong,
    ) {
        if handle != 0 {
            unsafe {
                drop(Box::from_raw(handle as *mut UnifiedAppHost));
            }
        }
    }
}
