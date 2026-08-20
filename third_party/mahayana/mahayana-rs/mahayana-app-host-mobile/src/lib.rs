use mahayana_app_host::{
    AppHost, AppHostFeatureMode, HostResponse, default_app_data_dir, dispatch_json,
};
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;

/// Creates a native app-host handle.
///
/// # Safety
/// If `app_data_dir` is non-null, it must point to a valid NUL-terminated C string
/// for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mahayana_app_host_create(app_data_dir: *const c_char) -> *mut AppHost {
    let path = if app_data_dir.is_null() {
        default_app_data_dir()
    } else {
        PathBuf::from(
            unsafe { CStr::from_ptr(app_data_dir) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    match AppHost::new(path) {
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
) -> *mut AppHost {
    let path = if app_data_dir.is_null() {
        default_app_data_dir()
    } else {
        PathBuf::from(
            unsafe { CStr::from_ptr(app_data_dir) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    match AppHost::new_with_feature_mode(path, AppHostFeatureMode::Test) {
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
    host: *mut AppHost,
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
pub unsafe extern "C" fn mahayana_app_host_destroy(host: *mut AppHost) {
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
    let output = match AppHost::new(default_app_data_dir()) {
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
    ) -> jlong {
        let path = match env.get_string(&app_data_dir) {
            Ok(value) => PathBuf::from(value.to_string_lossy().into_owned()),
            Err(_) => return 0,
        };
        match AppHost::new(path) {
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
        match AppHost::new_with_feature_mode(path, AppHostFeatureMode::Test) {
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
        let host = unsafe { &*(handle as *mut AppHost) };
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
                drop(Box::from_raw(handle as *mut AppHost));
            }
        }
    }
}
