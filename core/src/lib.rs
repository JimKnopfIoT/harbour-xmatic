//! xmatic-core — the Matrix protocol core, exposed to the Qt/QML front end
//! through a deliberately tiny C ABI.
//!
//! The whole surface is six functions: a handle is created, a callback is
//! registered, JSON commands go in, JSON messages come out. Everything else —
//! login, room list, timeline, encryption — is a message type, not a symbol.
//! That keeps the ABI stable while the Rust side follows matrix-rust-sdk
//! upstream. See `protocol.rs` for the message format.

// The dispatcher's futures nest deeply — every command arm contributes its own
// future to one enum — which overruns the default limit while computing layout.
#![recursion_limit = "512"]

mod call;
mod directory;
mod login;
mod media;
mod members;
mod profile;
mod private;
mod protocol;
mod markup;
mod recovery;
mod roomlist;
mod runtime;
mod session;
mod text;
mod timeline;
mod verification;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::mpsc;

use crate::protocol::{reply_error, Command};
use crate::runtime::Sink;

/// Opaque handle owned by the front end.
pub struct XmCore {
    /// Kept alive for as long as the core exists; dropping it stops all tasks.
    runtime: tokio::runtime::Runtime,
    commands: mpsc::UnboundedSender<Command>,
    sink: Arc<Sink>,
}

#[derive(Deserialize)]
struct CoreConfig {
    /// Directory the core may write to. Everything it persists lives below it.
    #[serde(rename = "dataDir")]
    data_dir: PathBuf,

    /// Directory for data that may be thrown away at any time.
    #[serde(rename = "cacheDir")]
    cache_dir: PathBuf,

    /// Base64 of the 32-byte store key from Sailfish Secrets. Absent when the
    /// secrets store is unavailable — the core then runs unencrypted rather
    /// than refusing to start. Never logged, wiped after decoding.
    #[serde(rename = "storeKey", default)]
    store_key: Option<String>,
}

/// Hands out a heap-allocated C string, or NULL if the value could not be
/// converted. Every pointer returned across the ABI comes from here, so
/// `xm_string_free` is always the correct way to release it.
fn into_c_string(value: String) -> *mut c_char {
    match CString::new(value) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Reads a borrowed UTF-8 string from the ABI.
///
/// # Safety
///
/// `pointer` must be NULL or a valid NUL-terminated string.
unsafe fn borrow_str<'a>(pointer: *const c_char) -> Option<&'a str> {
    if pointer.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(pointer) }.to_str().ok()
}

/// Version of the core library.
///
/// The caller owns the returned string and must release it with
/// `xm_string_free`. Returns NULL if the version could not be produced.
#[no_mangle]
pub extern "C" fn xm_version() -> *mut c_char {
    catch_unwind(AssertUnwindSafe(|| {
        into_c_string(format!("xmatic-core {}", env!("CARGO_PKG_VERSION")))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Creates a core instance.
///
/// `config_json` is an object with a `dataDir` member. Returns NULL if the
/// configuration could not be read or the runtime could not be started.
///
/// # Safety
///
/// `config_json` must be a valid NUL-terminated UTF-8 string. The returned
/// handle must eventually be released with `xm_core_free`.
#[no_mangle]
pub unsafe extern "C" fn xm_core_new(config_json: *const c_char) -> *mut XmCore {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let raw = unsafe { borrow_str(config_json) }?;
        let mut config: CoreConfig = serde_json::from_str(raw).ok()?;

        let paths = session::Paths::new(&config.data_dir, &config.cache_dir);
        paths.prepare().ok()?;

        let store_key = config
            .store_key
            .take()
            .and_then(|mut encoded| {
                let key = session::decode_key(&encoded);
                // The base64 copy is as much the key as the key itself.
                use zeroize::Zeroize;
                encoded.zeroize();
                key
            });

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("xmatic")
            .build()
            .ok()?;

        let sink = Arc::new(Sink::new());
        let commands = runtime::spawn(&runtime, paths, store_key, sink.clone());

        Some(Box::into_raw(Box::new(XmCore {
            runtime,
            commands,
            sink,
        })))
    }));

    result.ok().flatten().unwrap_or(std::ptr::null_mut())
}

/// Registers the function that receives every reply and event, as a JSON
/// string. It is called from a worker thread, so the front end must hop into
/// its own event loop before touching UI state.
///
/// The string passed to the callback is only valid for the duration of the
/// call and must be copied.
///
/// # Safety
///
/// `core` must be a handle from `xm_core_new`. `user_data` must stay valid
/// until the core is freed or another callback is registered.
#[no_mangle]
pub unsafe extern "C" fn xm_core_set_callback(
    core: *mut XmCore,
    callback: Option<extern "C" fn(*mut c_void, *const c_char)>,
    user_data: *mut c_void,
) {
    if core.is_null() {
        return;
    }
    let core = unsafe { &*core };
    core.sink.set_callback(callback, user_data);
}

/// Queues a command. Returns immediately; the reply arrives through the
/// callback.
///
/// # Safety
///
/// `core` must be a handle from `xm_core_new` and `command_json` a valid
/// NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xm_core_send(core: *mut XmCore, command_json: *const c_char) {
    if core.is_null() {
        return;
    }
    let core = unsafe { &*core };

    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(raw) = (unsafe { borrow_str(command_json) }) else {
            core.sink.emit(reply_error(0, "command was not valid UTF-8"));
            return;
        };

        // The id is parsed separately so a malformed command can still be
        // answered with the right id where one was given. Into a struct that
        // holds nothing but the id: a full `Value` would copy every field of
        // the command onto the heap unwiped — since the password login, one
        // of those fields can be a password.
        #[derive(serde::Deserialize)]
        struct CommandId {
            #[serde(default)]
            id: u64,
        }
        let id = serde_json::from_str::<CommandId>(raw)
            .map(|command| command.id)
            .unwrap_or(0);

        match serde_json::from_str::<Command>(raw) {
            Ok(command) => {
                if core.commands.send(command).is_err() {
                    core.sink.emit(reply_error(id, "core is shutting down"));
                }
            }
            Err(error) => {
                core.sink
                    .emit(reply_error(id, format!("command not understood: {error}")));
            }
        }
    }));
}

/// Releases the core and stops its runtime.
///
/// # Safety
///
/// `core` must be a handle from `xm_core_new` and must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn xm_core_free(core: *mut XmCore) {
    if core.is_null() {
        return;
    }
    let core = unsafe { Box::from_raw(core) };
    // Stop delivering into a front end that is going away, then let the
    // runtime wind down without blocking the caller's thread.
    core.sink.set_callback(None, std::ptr::null_mut());
    let XmCore {
        runtime, commands, ..
    } = *core;
    drop(commands);
    runtime.shutdown_background();
}

/// Releases a string previously returned by this library.
///
/// # Safety
///
/// `s` must be NULL or a pointer returned by one of this library's functions,
/// and must not be released twice.
#[no_mangle]
pub unsafe extern "C" fn xm_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { CString::from_raw(s) });
    }));
}
