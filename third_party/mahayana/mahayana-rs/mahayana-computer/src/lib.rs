//! Shared local-computer executor.
//!
//! The desktop UI, paired mobile clients, and the local AI all use this exact
//! action contract. The caller remains responsible for consent, authentication,
//! policy, and audit decisions; this crate only executes already-authorized
//! actions on the machine where Fabushi is installed.

use base64::Engine as _;
use mahayana_host_protocol::COMPUTER_MAX_ACTIONS_PER_CALL;
use mahayana_host_protocol::COMPUTER_MAX_WAIT_MS;
use mahayana_host_protocol::ComputerAction;
use mahayana_host_protocol::ComputerActionKind;
use mahayana_host_protocol::ComputerActionResult;
use mahayana_host_protocol::ComputerControlOrigin;
use mahayana_host_protocol::ComputerSnapshot;
use mahayana_host_protocol::ComputerStatus;
use mahayana_host_protocol::LocalToolPermission;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const FINAL_SCREEN_SETTLE_MS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputerControlPolicy {
    pub local_execution_enabled: bool,
    pub remote_control_enabled: bool,
    pub ai_control_enabled: bool,
    pub local_tool_permission: LocalToolPermission,
}

impl Default for ComputerControlPolicy {
    fn default() -> Self {
        Self {
            local_execution_enabled: true,
            remote_control_enabled: false,
            ai_control_enabled: true,
            local_tool_permission: LocalToolPermission::Ask,
        }
    }
}

static CONTROL_POLICY: LazyLock<RwLock<ComputerControlPolicy>> =
    LazyLock::new(|| RwLock::new(ComputerControlPolicy::default()));
static EXECUTION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
/// Increments as soon as a human-origin action is requested, even before that
/// action acquires the desktop mutex. AI batches check this between actions (and
/// during waits), so the user can always take the real computer back promptly.
static USER_OVERRIDE_EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn set_control_policy(policy: ComputerControlPolicy) {
    if let Ok(mut current) = CONTROL_POLICY.write() {
        *current = policy;
    }
}

pub fn control_policy() -> ComputerControlPolicy {
    CONTROL_POLICY
        .read()
        .map(|policy| *policy)
        .unwrap_or_default()
}

#[derive(Debug, thiserror::Error)]
pub enum ComputerError {
    #[error("computer control is unavailable on this platform")]
    Unavailable,
    #[error("computer action is invalid: {0}")]
    InvalidAction(String),
    #[error("computer permission is required: {0}")]
    Permission(String),
    #[error("computer capture failed: {0}")]
    Capture(String),
    #[error("computer input failed: {0}")]
    Input(String),
    #[error("AI computer control was preempted by the user")]
    Preempted,
    #[error("computer I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub fn status(
    local_execution_enabled: bool,
    route_egress_locally: bool,
    remote_control_enabled: bool,
    ai_control_enabled: bool,
) -> ComputerStatus {
    #[cfg(target_os = "macos")]
    let (
        available,
        capture_supported,
        input_supported,
        accessibility_granted,
        screen_recording_granted,
    ) = (
        true,
        true,
        true,
        macos::accessibility_granted(),
        macos::screen_recording_granted(),
    );
    #[cfg(target_os = "windows")]
    let (
        available,
        capture_supported,
        input_supported,
        accessibility_granted,
        screen_recording_granted,
    ) = (true, true, true, true, true);
    #[cfg(target_os = "linux")]
    let (
        available,
        capture_supported,
        input_supported,
        accessibility_granted,
        screen_recording_granted,
    ) = {
        let display = std::env::var_os("DISPLAY").is_some();
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let desktop = display || wayland;
        (desktop, desktop, desktop, desktop, desktop)
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let (
        available,
        capture_supported,
        input_supported,
        accessibility_granted,
        screen_recording_granted,
    ) = (false, false, false, false, false);

    ComputerStatus {
        platform: std::env::consts::OS.into(),
        available,
        capture_supported,
        input_supported,
        accessibility_granted,
        screen_recording_granted,
        local_execution_enabled,
        route_egress_locally,
        remote_control_enabled,
        ai_control_enabled,
    }
}

pub fn capture_screen() -> Result<ComputerSnapshot, ComputerError> {
    let _lease = EXECUTION_LOCK
        .lock()
        .map_err(|_| ComputerError::Input("computer execution lock is poisoned".into()))?;
    #[cfg(target_os = "macos")]
    return macos::capture_screen();
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    return portable::capture_screen();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err(ComputerError::Unavailable)
}

pub fn execute(
    actions: &[ComputerAction],
    origin: ComputerControlOrigin,
) -> Result<ComputerActionResult, ComputerError> {
    if actions.is_empty() {
        return Err(ComputerError::InvalidAction(
            "at least one action is required".into(),
        ));
    }
    if actions.len() > COMPUTER_MAX_ACTIONS_PER_CALL {
        return Err(ComputerError::InvalidAction(format!(
            "at most {COMPUTER_MAX_ACTIONS_PER_CALL} actions may be batched"
        )));
    }
    for (index, action) in actions.iter().enumerate() {
        validate_action(action)?;
        if index > 0 && action.action == ComputerActionKind::Screenshot {
            return Err(ComputerError::InvalidAction(
                "screenshot is only allowed as the primary action, not inside then".into(),
            ));
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let ai_epoch = USER_OVERRIDE_EPOCH.load(Ordering::SeqCst);
        if origin != ComputerControlOrigin::Ai {
            USER_OVERRIDE_EPOCH.fetch_add(1, Ordering::SeqCst);
        }
        let _lease = EXECUTION_LOCK
            .lock()
            .map_err(|_| ComputerError::Input("computer execution lock is poisoned".into()))?;
        for action in actions {
            if origin == ComputerControlOrigin::Ai {
                ensure_ai_not_preempted(ai_epoch)?;
                if action.action == ComputerActionKind::Wait {
                    ai_wait_with_preemption(action.wait_ms.unwrap_or(1_000), ai_epoch)?;
                    continue;
                }
            }
            #[cfg(target_os = "macos")]
            macos::execute_action(action)?;
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            portable::execute_action(action)?;
            if origin == ComputerControlOrigin::Ai {
                ensure_ai_not_preempted(ai_epoch)?;
            }
        }
        std::thread::sleep(Duration::from_millis(FINAL_SCREEN_SETTLE_MS));
        #[cfg(target_os = "macos")]
        let snapshot = macos::capture_screen()?;
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        let snapshot = portable::capture_screen()?;
        Ok(ComputerActionResult {
            origin,
            actions_executed: actions.len(),
            snapshot,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = origin;
        Err(ComputerError::Unavailable)
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn ensure_ai_not_preempted(epoch: u64) -> Result<(), ComputerError> {
    if USER_OVERRIDE_EPOCH.load(Ordering::SeqCst) != epoch {
        Err(ComputerError::Preempted)
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn ai_wait_with_preemption(wait_ms: u64, epoch: u64) -> Result<(), ComputerError> {
    let mut remaining = wait_ms;
    while remaining > 0 {
        ensure_ai_not_preempted(epoch)?;
        let slice = remaining.min(100);
        std::thread::sleep(Duration::from_millis(slice));
        remaining -= slice;
    }
    ensure_ai_not_preempted(epoch)
}

pub fn validate_action(action: &ComputerAction) -> Result<(), ComputerError> {
    let pair = |left: Option<i32>, right: Option<i32>, label: &str| {
        if left.is_some() != right.is_some() {
            Err(ComputerError::InvalidAction(format!(
                "{label} coordinates must be provided together"
            )))
        } else {
            Ok(())
        }
    };
    pair(action.x, action.y, "x/y")?;
    pair(action.x2, action.y2, "x2/y2")?;
    match action.action {
        ComputerActionKind::Screenshot => {}
        ComputerActionKind::Click => {
            if action
                .click_count
                .is_some_and(|count| !(1..=3).contains(&count))
            {
                return Err(ComputerError::InvalidAction(
                    "count must be 1, 2, or 3".into(),
                ));
            }
        }
        ComputerActionKind::Move => {}
        ComputerActionKind::Drag => {
            let has_path = action.path.as_ref().is_some_and(|path| path.len() >= 2);
            let has_endpoints = action.x.is_some()
                && action.y.is_some()
                && action.x2.is_some()
                && action.y2.is_some();
            if !has_path && !has_endpoints {
                return Err(ComputerError::InvalidAction(
                    "drag requires path with at least two points or x/y/x2/y2".into(),
                ));
            }
        }
        ComputerActionKind::Type => {
            if action.text.is_none() {
                return Err(ComputerError::InvalidAction("type requires text".into()));
            }
        }
        ComputerActionKind::Key => {
            if action
                .key
                .as_deref()
                .is_none_or(|key| key.trim().is_empty())
            {
                return Err(ComputerError::InvalidAction(
                    "key requires a key or chord".into(),
                ));
            }
        }
        ComputerActionKind::Scroll => {}
        ComputerActionKind::Wait => {
            if action
                .wait_ms
                .is_some_and(|wait_ms| wait_ms > COMPUTER_MAX_WAIT_MS)
            {
                return Err(ComputerError::InvalidAction(format!(
                    "durationMs must be at most {COMPUTER_MAX_WAIT_MS}"
                )));
            }
        }
    }
    Ok(())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
fn png_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return (None, None);
    }
    (
        Some(u32::from_be_bytes(bytes[16..20].try_into().unwrap())),
        Some(u32::from_be_bytes(bytes[20..24].try_into().unwrap())),
    )
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
mod portable {
    use super::*;
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
    use image::{DynamicImage, ImageFormat};
    use mahayana_host_protocol::{ComputerMouseButton, ComputerPoint, ComputerScrollDirection};
    use std::io::Cursor;
    use xcap::Monitor;

    fn enigo() -> Result<Enigo, ComputerError> {
        Enigo::new(&Settings::default()).map_err(|error| ComputerError::Input(error.to_string()))
    }

    fn position(action: &ComputerAction, enigo: &Enigo) -> Result<(i32, i32), ComputerError> {
        if let (Some(x), Some(y)) = (action.x, action.y) {
            Ok((x, y))
        } else {
            enigo
                .location()
                .map_err(|error| ComputerError::Input(error.to_string()))
        }
    }

    fn button(value: Option<ComputerMouseButton>) -> Button {
        match value.unwrap_or(ComputerMouseButton::Left) {
            ComputerMouseButton::Left => Button::Left,
            ComputerMouseButton::Right => Button::Right,
            ComputerMouseButton::Middle => Button::Middle,
        }
    }

    fn move_to(enigo: &mut Enigo, x: i32, y: i32) -> Result<(), ComputerError> {
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|error| ComputerError::Input(error.to_string()))
    }

    fn drag_points(action: &ComputerAction) -> Result<Vec<ComputerPoint>, ComputerError> {
        if let Some(path) = action.path.as_ref().filter(|path| path.len() >= 2) {
            return Ok(path.clone());
        }
        match (action.x, action.y, action.x2, action.y2) {
            (Some(x), Some(y), Some(x2), Some(y2)) => {
                Ok(vec![ComputerPoint { x, y }, ComputerPoint { x: x2, y: y2 }])
            }
            _ => Err(ComputerError::InvalidAction(
                "drag requires a path or x/y/x2/y2".into(),
            )),
        }
    }

    fn named_key(raw: &str) -> Result<Key, ComputerError> {
        let normalized = raw.trim().to_ascii_lowercase();
        let key = match normalized.as_str() {
            "ctrl" | "control" => Key::Control,
            "shift" => Key::Shift,
            "alt" | "option" => Key::Alt,
            "meta" | "cmd" | "command" | "super" | "win" | "windows" => Key::Meta,
            "enter" | "return" => Key::Return,
            "tab" => Key::Tab,
            "escape" | "esc" => Key::Escape,
            "backspace" => Key::Backspace,
            "delete" | "del" => Key::Delete,
            "space" => Key::Space,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" | "page-up" => Key::PageUp,
            "pagedown" | "page-down" => Key::PageDown,
            "arrowup" | "up" => Key::UpArrow,
            "arrowdown" | "down" => Key::DownArrow,
            "arrowleft" | "left" => Key::LeftArrow,
            "arrowright" | "right" => Key::RightArrow,
            "f1" => Key::F1,
            "f2" => Key::F2,
            "f3" => Key::F3,
            "f4" => Key::F4,
            "f5" => Key::F5,
            "f6" => Key::F6,
            "f7" => Key::F7,
            "f8" => Key::F8,
            "f9" => Key::F9,
            "f10" => Key::F10,
            "f11" => Key::F11,
            "f12" => Key::F12,
            _ if normalized.chars().count() == 1 => {
                Key::Unicode(normalized.chars().next().unwrap())
            }
            _ => {
                return Err(ComputerError::InvalidAction(format!(
                    "unsupported key: {raw}"
                )));
            }
        };
        Ok(key)
    }

    fn press_chord(enigo: &mut Enigo, raw: &str) -> Result<(), ComputerError> {
        let parts: Vec<&str> = raw
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            return Err(ComputerError::InvalidAction("key chord is empty".into()));
        }
        let mut keys = Vec::with_capacity(parts.len());
        for part in parts {
            keys.push(named_key(part)?);
        }
        if keys.len() == 1 {
            return enigo
                .key(keys[0], Direction::Click)
                .map_err(|error| ComputerError::Input(error.to_string()));
        }
        for key in &keys[..keys.len() - 1] {
            enigo
                .key(*key, Direction::Press)
                .map_err(|error| ComputerError::Input(error.to_string()))?;
        }
        enigo
            .key(keys[keys.len() - 1], Direction::Click)
            .map_err(|error| ComputerError::Input(error.to_string()))?;
        for key in keys[..keys.len() - 1].iter().rev() {
            enigo
                .key(*key, Direction::Release)
                .map_err(|error| ComputerError::Input(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) fn execute_action(action: &ComputerAction) -> Result<(), ComputerError> {
        let mut enigo = enigo()?;
        match action.action {
            ComputerActionKind::Screenshot => Ok(()),
            ComputerActionKind::Click => {
                let (x, y) = position(action, &enigo)?;
                move_to(&mut enigo, x, y)?;
                for _ in 0..action.click_count.unwrap_or(1) {
                    enigo
                        .button(button(action.button), Direction::Click)
                        .map_err(|error| ComputerError::Input(error.to_string()))?;
                }
                Ok(())
            }
            ComputerActionKind::Move => {
                let (x, y) = position(action, &enigo)?;
                move_to(&mut enigo, x, y)
            }
            ComputerActionKind::Drag => {
                let points = drag_points(action)?;
                let first = &points[0];
                move_to(&mut enigo, first.x, first.y)?;
                enigo
                    .button(button(action.button), Direction::Press)
                    .map_err(|error| ComputerError::Input(error.to_string()))?;
                let duration = action.wait_ms.unwrap_or(300).max(1);
                let slices = u64::try_from(points.len().saturating_sub(1))
                    .unwrap_or(1)
                    .max(1);
                let delay = Duration::from_millis((duration / slices).max(1));
                for point in points.iter().skip(1) {
                    move_to(&mut enigo, point.x, point.y)?;
                    std::thread::sleep(delay);
                }
                enigo
                    .button(button(action.button), Direction::Release)
                    .map_err(|error| ComputerError::Input(error.to_string()))
            }
            ComputerActionKind::Type => enigo
                .text(action.text.as_deref().unwrap_or_default())
                .map_err(|error| ComputerError::Input(error.to_string())),
            ComputerActionKind::Key => {
                press_chord(&mut enigo, action.key.as_deref().unwrap_or_default())
            }
            ComputerActionKind::Scroll => {
                if let (Some(x), Some(y)) = (action.x, action.y) {
                    move_to(&mut enigo, x, y)?;
                }
                let magnitude = action.amount.unwrap_or(1).max(1);
                let (axis, value) = match action.direction.unwrap_or(ComputerScrollDirection::Down)
                {
                    ComputerScrollDirection::Up => (Axis::Vertical, -magnitude),
                    ComputerScrollDirection::Down => (Axis::Vertical, magnitude),
                    ComputerScrollDirection::Left => (Axis::Horizontal, -magnitude),
                    ComputerScrollDirection::Right => (Axis::Horizontal, magnitude),
                };
                enigo
                    .scroll(value, axis)
                    .map_err(|error| ComputerError::Input(error.to_string()))
            }
            ComputerActionKind::Wait => {
                std::thread::sleep(Duration::from_millis(action.wait_ms.unwrap_or(1_000)));
                Ok(())
            }
        }
    }

    pub(super) fn capture_screen() -> Result<ComputerSnapshot, ComputerError> {
        let monitors = Monitor::all().map_err(|error| ComputerError::Capture(error.to_string()))?;
        let monitor = monitors
            .iter()
            .find(|monitor| monitor.is_primary().unwrap_or(false))
            .or_else(|| monitors.first())
            .ok_or_else(|| ComputerError::Capture("no monitor is available".into()))?;
        let image = monitor
            .capture_image()
            .map_err(|error| ComputerError::Capture(error.to_string()))?;
        let width = image.width();
        let height = image.height();
        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut cursor, ImageFormat::Png)
            .map_err(|error| ComputerError::Capture(error.to_string()))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(cursor.into_inner());
        Ok(ComputerSnapshot {
            captured_at_ms: now_millis(),
            data_url: format!("data:image/png;base64,{encoded}"),
            width: Some(width),
            height: Some(height),
        })
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use core_foundation::base::TCFType;
    use core_foundation::data::CFData;
    use core_foundation::string::CFString;
    use core_foundation_sys::base::CFRelease;
    use core_foundation_sys::base::CFTypeRef;
    use core_foundation_sys::base::kCFAllocatorDefault;
    use core_foundation_sys::data::CFDataCreateMutable;
    use core_foundation_sys::data::CFDataRef;
    use core_foundation_sys::data::CFMutableDataRef;
    use core_foundation_sys::dictionary::CFDictionaryRef;
    use core_foundation_sys::string::CFStringRef;
    use core_graphics::display::CGDisplay;
    use core_graphics::event::CGEvent;
    use core_graphics::event::CGEventFlags;
    use core_graphics::event::CGEventTapLocation;
    use core_graphics::event::CGEventType;
    use core_graphics::event::CGMouseButton;
    use core_graphics::event::EventField;
    use core_graphics::event::KeyCode;
    use core_graphics::event::ScrollEventUnit;
    use core_graphics::event_source::CGEventSource;
    use core_graphics::event_source::CGEventSourceStateID;
    use core_graphics::geometry::CGPoint;
    use foreign_types::ForeignType;
    use mahayana_host_protocol::ComputerMouseButton;
    use mahayana_host_protocol::ComputerPoint;
    use mahayana_host_protocol::ComputerScrollDirection;
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageDestinationCreateWithData(
            data: CFMutableDataRef,
            type_identifier: CFStringRef,
            count: usize,
            options: CFDictionaryRef,
        ) -> *mut c_void;
        fn CGImageDestinationAddImage(
            destination: *mut c_void,
            image: *mut c_void,
            properties: CFDictionaryRef,
        );
        fn CGImageDestinationFinalize(destination: *mut c_void) -> u8;
    }

    pub(super) fn accessibility_granted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub(super) fn screen_recording_granted() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    fn event_source() -> Result<CGEventSource, ComputerError> {
        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| ComputerError::Input("could not create CoreGraphics event source".into()))
    }

    pub(super) fn capture_screen() -> Result<ComputerSnapshot, ComputerError> {
        if !screen_recording_granted() {
            return Err(ComputerError::Permission(
                "Screen Recording permission is required in System Settings > Privacy & Security"
                    .into(),
            ));
        }
        let image = CGDisplay::main().image().ok_or_else(|| {
            ComputerError::Capture("CoreGraphics could not capture the main display".into())
        })?;
        let width = u32::try_from(image.width()).ok();
        let height = u32::try_from(image.height()).ok();
        let data_ref = unsafe { CFDataCreateMutable(kCFAllocatorDefault, 0) };
        if data_ref.is_null() {
            return Err(ComputerError::Capture(
                "ImageIO could not allocate the encoded screen buffer".into(),
            ));
        }
        let type_identifier = CFString::new("public.jpeg");
        let destination = unsafe {
            CGImageDestinationCreateWithData(
                data_ref,
                type_identifier.as_concrete_TypeRef(),
                1,
                ptr::null(),
            )
        };
        if destination.is_null() {
            unsafe { CFRelease(data_ref as CFTypeRef) };
            return Err(ComputerError::Capture(
                "ImageIO could not create the screen encoder".into(),
            ));
        }
        unsafe {
            CGImageDestinationAddImage(destination, image.as_ptr() as *mut c_void, ptr::null());
        }
        let finalized = unsafe { CGImageDestinationFinalize(destination) };
        unsafe { CFRelease(destination as CFTypeRef) };
        if finalized == 0 {
            unsafe { CFRelease(data_ref as CFTypeRef) };
            return Err(ComputerError::Capture(
                "ImageIO could not finalize the encoded screen".into(),
            ));
        }
        let data = unsafe { CFData::wrap_under_create_rule(data_ref as CFDataRef) };
        if data.is_empty() {
            return Err(ComputerError::Capture("screen capture was empty".into()));
        }
        Ok(ComputerSnapshot {
            captured_at_ms: now_millis(),
            data_url: format!(
                "data:image/jpeg;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data.bytes())
            ),
            width,
            height,
        })
    }

    pub(super) fn execute_action(action: &ComputerAction) -> Result<(), ComputerError> {
        if action.action != ComputerActionKind::Screenshot
            && action.action != ComputerActionKind::Wait
            && !accessibility_granted()
        {
            return Err(ComputerError::Permission(
                "Accessibility permission is required in System Settings > Privacy & Security"
                    .into(),
            ));
        }
        match action.action {
            ComputerActionKind::Screenshot => Ok(()),
            ComputerActionKind::Click => click(action),
            ComputerActionKind::Move => move_pointer(action),
            ComputerActionKind::Drag => drag(action),
            ComputerActionKind::Type => type_text(action.text.as_deref().unwrap_or_default()),
            ComputerActionKind::Key => press_key(action.key.as_deref().unwrap_or_default()),
            ComputerActionKind::Scroll => scroll(action),
            ComputerActionKind::Wait => {
                std::thread::sleep(Duration::from_millis(action.wait_ms.unwrap_or(1_000)));
                Ok(())
            }
        }
    }

    fn cursor_position() -> Result<CGPoint, ComputerError> {
        CGEvent::new(event_source()?)
            .map(|event| event.location())
            .map_err(|_| ComputerError::Input("could not read cursor location".into()))
    }

    fn action_position(action: &ComputerAction) -> Result<CGPoint, ComputerError> {
        match (action.x, action.y) {
            (Some(x), Some(y)) => Ok(CGPoint::new(x as f64, y as f64)),
            (None, None) => cursor_position(),
            _ => Err(ComputerError::InvalidAction(
                "x/y coordinates must be provided together".into(),
            )),
        }
    }

    fn button(value: Option<ComputerMouseButton>) -> CGMouseButton {
        match value.unwrap_or(ComputerMouseButton::Left) {
            ComputerMouseButton::Left => CGMouseButton::Left,
            ComputerMouseButton::Right => CGMouseButton::Right,
            ComputerMouseButton::Middle => CGMouseButton::Center,
        }
    }

    fn mouse_types(button: CGMouseButton) -> (CGEventType, CGEventType, CGEventType) {
        match button {
            CGMouseButton::Left => (
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseDragged,
                CGEventType::LeftMouseUp,
            ),
            CGMouseButton::Right => (
                CGEventType::RightMouseDown,
                CGEventType::RightMouseDragged,
                CGEventType::RightMouseUp,
            ),
            CGMouseButton::Center => (
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseDragged,
                CGEventType::OtherMouseUp,
            ),
        }
    }

    fn post_mouse(
        event_type: CGEventType,
        position: CGPoint,
        mouse_button: CGMouseButton,
        click_count: Option<u8>,
    ) -> Result<(), ComputerError> {
        let event = CGEvent::new_mouse_event(event_source()?, event_type, position, mouse_button)
            .map_err(|_| ComputerError::Input("could not create mouse event".into()))?;
        if let Some(count) = click_count {
            event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, i64::from(count));
        }
        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn move_to(position: CGPoint, dragged: Option<CGMouseButton>) -> Result<(), ComputerError> {
        let event_type = dragged.map_or(CGEventType::MouseMoved, |button| mouse_types(button).1);
        post_mouse(
            event_type,
            position,
            dragged.unwrap_or(CGMouseButton::Left),
            None,
        )
    }

    fn move_pointer(action: &ComputerAction) -> Result<(), ComputerError> {
        let position = action_position(action)?;
        move_to(position, None)
    }

    fn click(action: &ComputerAction) -> Result<(), ComputerError> {
        let position = action_position(action)?;
        let mouse_button = button(action.button);
        let (down, _, up) = mouse_types(mouse_button);
        let count = action.click_count.unwrap_or(1).clamp(1, 3);
        move_to(position, None)?;
        for click_number in 1..=count {
            post_mouse(down, position, mouse_button, Some(click_number))?;
            post_mouse(up, position, mouse_button, Some(click_number))?;
            if click_number < count {
                std::thread::sleep(Duration::from_millis(60));
            }
        }
        Ok(())
    }

    fn drag_points(action: &ComputerAction) -> Result<Vec<ComputerPoint>, ComputerError> {
        if let Some(path) = action.path.as_ref().filter(|path| path.len() >= 2) {
            return Ok(path.clone());
        }
        match (action.x, action.y, action.x2, action.y2) {
            (Some(x), Some(y), Some(x2), Some(y2)) => {
                Ok(vec![ComputerPoint { x, y }, ComputerPoint { x: x2, y: y2 }])
            }
            _ => Err(ComputerError::InvalidAction(
                "drag requires path or x/y/x2/y2".into(),
            )),
        }
    }

    fn drag(action: &ComputerAction) -> Result<(), ComputerError> {
        let points = drag_points(action)?;
        let mouse_button = button(action.button);
        let (down, _, up) = mouse_types(mouse_button);
        let first = CGPoint::new(points[0].x as f64, points[0].y as f64);
        move_to(first, None)?;
        post_mouse(down, first, mouse_button, Some(1))?;
        std::thread::sleep(Duration::from_millis(20));
        for point in points.iter().skip(1) {
            move_to(
                CGPoint::new(point.x as f64, point.y as f64),
                Some(mouse_button),
            )?;
            std::thread::sleep(Duration::from_millis(12));
        }
        let last = points.last().expect("validated drag points");
        post_mouse(
            up,
            CGPoint::new(last.x as f64, last.y as f64),
            mouse_button,
            Some(1),
        )
    }

    fn type_text(text: &str) -> Result<(), ComputerError> {
        if text.is_empty() {
            return Ok(());
        }
        // CoreGraphics Unicode keyboard events preserve arbitrary user text and
        // avoid clipboard mutation, so remote/mobile typing has the same semantics
        // as AI typing without exposing the user's clipboard.
        for chunk in unicode_chunks(text, 20) {
            let down = CGEvent::new_keyboard_event(event_source()?, 0, true)
                .map_err(|_| ComputerError::Input("could not create keyboard event".into()))?;
            down.set_string(&chunk);
            down.post(CGEventTapLocation::HID);
            let up = CGEvent::new_keyboard_event(event_source()?, 0, false)
                .map_err(|_| ComputerError::Input("could not create keyboard event".into()))?;
            up.post(CGEventTapLocation::HID);
        }
        Ok(())
    }

    fn unicode_chunks(text: &str, max_utf16: usize) -> Vec<String> {
        let mut chunks = Vec::new();
        let mut current = String::new();
        let mut units = 0usize;
        for ch in text.chars() {
            let next = ch.len_utf16();
            if !current.is_empty() && units + next > max_utf16 {
                chunks.push(std::mem::take(&mut current));
                units = 0;
            }
            current.push(ch);
            units += next;
        }
        if !current.is_empty() {
            chunks.push(current);
        }
        chunks
    }

    fn press_key(chord: &str) -> Result<(), ComputerError> {
        let parts = chord
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let Some(raw_key) = parts.last() else {
            return Err(ComputerError::InvalidAction("empty key chord".into()));
        };
        let mut flags = CGEventFlags::empty();
        for modifier in &parts[..parts.len().saturating_sub(1)] {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
                "alt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
                "shift" => flags |= CGEventFlags::CGEventFlagShift,
                "cmd" | "command" | "meta" | "super" => flags |= CGEventFlags::CGEventFlagCommand,
                other => {
                    return Err(ComputerError::InvalidAction(format!(
                        "unsupported modifier: {other}"
                    )));
                }
            }
        }
        let keycode = key_code(raw_key)
            .ok_or_else(|| ComputerError::InvalidAction(format!("unsupported key: {raw_key}")))?;
        let down = CGEvent::new_keyboard_event(event_source()?, keycode, true)
            .map_err(|_| ComputerError::Input("could not create key-down event".into()))?;
        down.set_flags(flags);
        down.post(CGEventTapLocation::HID);
        let up = CGEvent::new_keyboard_event(event_source()?, keycode, false)
            .map_err(|_| ComputerError::Input("could not create key-up event".into()))?;
        up.set_flags(flags);
        up.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn key_code(raw: &str) -> Option<u16> {
        let key = raw.to_ascii_lowercase();
        Some(match key.as_str() {
            "a" => KeyCode::ANSI_A,
            "b" => KeyCode::ANSI_B,
            "c" => KeyCode::ANSI_C,
            "d" => KeyCode::ANSI_D,
            "e" => KeyCode::ANSI_E,
            "f" => KeyCode::ANSI_F,
            "g" => KeyCode::ANSI_G,
            "h" => KeyCode::ANSI_H,
            "i" => KeyCode::ANSI_I,
            "j" => KeyCode::ANSI_J,
            "k" => KeyCode::ANSI_K,
            "l" => KeyCode::ANSI_L,
            "m" => KeyCode::ANSI_M,
            "n" => KeyCode::ANSI_N,
            "o" => KeyCode::ANSI_O,
            "p" => KeyCode::ANSI_P,
            "q" => KeyCode::ANSI_Q,
            "r" => KeyCode::ANSI_R,
            "s" => KeyCode::ANSI_S,
            "t" => KeyCode::ANSI_T,
            "u" => KeyCode::ANSI_U,
            "v" => KeyCode::ANSI_V,
            "w" => KeyCode::ANSI_W,
            "x" => KeyCode::ANSI_X,
            "y" => KeyCode::ANSI_Y,
            "z" => KeyCode::ANSI_Z,
            "0" => KeyCode::ANSI_0,
            "1" => KeyCode::ANSI_1,
            "2" => KeyCode::ANSI_2,
            "3" => KeyCode::ANSI_3,
            "4" => KeyCode::ANSI_4,
            "5" => KeyCode::ANSI_5,
            "6" => KeyCode::ANSI_6,
            "7" => KeyCode::ANSI_7,
            "8" => KeyCode::ANSI_8,
            "9" => KeyCode::ANSI_9,
            "return" | "enter" => KeyCode::RETURN,
            "tab" => KeyCode::TAB,
            "space" | "spacebar" => KeyCode::SPACE,
            "backspace" => KeyCode::DELETE,
            "delete" | "forwarddelete" => KeyCode::FORWARD_DELETE,
            "escape" | "esc" => KeyCode::ESCAPE,
            "left" | "arrowleft" => KeyCode::LEFT_ARROW,
            "right" | "arrowright" => KeyCode::RIGHT_ARROW,
            "up" | "arrowup" => KeyCode::UP_ARROW,
            "down" | "arrowdown" => KeyCode::DOWN_ARROW,
            "home" => KeyCode::HOME,
            "end" => KeyCode::END,
            "pageup" => KeyCode::PAGE_UP,
            "pagedown" => KeyCode::PAGE_DOWN,
            "f1" => KeyCode::F1,
            "f2" => KeyCode::F2,
            "f3" => KeyCode::F3,
            "f4" => KeyCode::F4,
            "f5" => KeyCode::F5,
            "f6" => KeyCode::F6,
            "f7" => KeyCode::F7,
            "f8" => KeyCode::F8,
            "f9" => KeyCode::F9,
            "f10" => KeyCode::F10,
            "f11" => KeyCode::F11,
            "f12" => KeyCode::F12,
            "-" | "minus" => KeyCode::ANSI_MINUS,
            "=" | "equal" => KeyCode::ANSI_EQUAL,
            "," | "comma" => KeyCode::ANSI_COMMA,
            "." | "period" => KeyCode::ANSI_PERIOD,
            "/" | "slash" => KeyCode::ANSI_SLASH,
            ";" | "semicolon" => KeyCode::ANSI_SEMICOLON,
            "'" | "quote" => KeyCode::ANSI_QUOTE,
            "[" | "leftbracket" => KeyCode::ANSI_LEFT_BRACKET,
            "]" | "rightbracket" => KeyCode::ANSI_RIGHT_BRACKET,
            "\\" | "backslash" => KeyCode::ANSI_BACKSLASH,
            "`" | "grave" => KeyCode::ANSI_GRAVE,
            _ => return None,
        })
    }

    fn scroll(action: &ComputerAction) -> Result<(), ComputerError> {
        if action.x.is_some() {
            move_to(action_position(action)?, None)?;
        }
        let amount = action.amount.unwrap_or(3).abs().max(1);
        let (vertical, horizontal) = match action.direction.unwrap_or(ComputerScrollDirection::Down)
        {
            ComputerScrollDirection::Up => (amount, 0),
            ComputerScrollDirection::Down => (-amount, 0),
            ComputerScrollDirection::Left => (0, amount),
            ComputerScrollDirection::Right => (0, -amount),
        };
        let event = CGEvent::new_scroll_event(
            event_source()?,
            ScrollEventUnit::LINE,
            2,
            vertical,
            horizontal,
            0,
        )
        .map_err(|_| ComputerError::Input("could not create scroll event".into()))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahayana_host_protocol::ComputerActionKind;

    fn action(kind: ComputerActionKind) -> ComputerAction {
        ComputerAction {
            action: kind,
            x: None,
            y: None,
            x2: None,
            y2: None,
            path: None,
            text: None,
            key: None,
            button: None,
            click_count: None,
            direction: None,
            amount: None,
            wait_ms: None,
            description: None,
        }
    }

    #[test]
    fn action_limits_are_enforced_without_touching_the_desktop() {
        let mut wait = action(ComputerActionKind::Wait);
        wait.wait_ms = Some(COMPUTER_MAX_WAIT_MS + 1);
        assert!(validate_action(&wait).is_err());

        let mut drag = action(ComputerActionKind::Drag);
        drag.x = Some(10);
        drag.y = Some(20);
        assert!(validate_action(&drag).is_err());

        let mut click = action(ComputerActionKind::Click);
        click.click_count = Some(4);
        assert!(validate_action(&click).is_err());
    }

    #[test]
    fn follow_up_sequence_rejects_screenshot_before_touching_the_desktop() {
        let primary = action(ComputerActionKind::Click);
        let follow_up = action(ComputerActionKind::Screenshot);
        let error = execute(&[primary, follow_up], ComputerControlOrigin::LocalUi)
            .expect_err("follow-up screenshot must be rejected before desktop execution");
        assert!(error.to_string().contains("primary action"));
    }

    #[test]
    fn human_override_epoch_preempts_ai_without_touching_the_desktop() {
        let epoch = USER_OVERRIDE_EPOCH.load(Ordering::SeqCst);
        USER_OVERRIDE_EPOCH.fetch_add(1, Ordering::SeqCst);
        assert!(matches!(
            ensure_ai_not_preempted(epoch),
            Err(ComputerError::Preempted)
        ));
    }

    #[test]
    fn png_dimensions_reads_ihdr_without_image_dependencies() {
        let mut bytes = vec![0u8; 24];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&1440u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&900u32.to_be_bytes());
        assert_eq!(png_dimensions(&bytes), (Some(1440), Some(900)));
    }
}
