use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
};

use crate::{
    error::{Error, Result},
    pathutil::path_to_cstring,
};

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_create_params() -> *mut c_void;
    fn sandbox_set_param(params: *mut c_void, key: *const c_char, value: *const c_char) -> c_int;
    fn sandbox_free_params(params: *mut c_void);
    fn sandbox_compile_string(
        profile: *const c_char,
        params: *mut c_void,
        errorbuf: *mut *mut c_char,
    ) -> *mut c_void;
    fn sandbox_free_profile(profile: *mut c_void);
    fn sandbox_apply(profile: *mut c_void) -> c_int;
}

pub fn apply(
    profile_text: &str,
    workdir: &Path,
    tty: &Path,
    histfile: Option<&Path>,
) -> Result<()> {
    let mut params = Params::new()?;
    params.set("WORKDIR", path_to_cstring(workdir)?)?;
    params.set("TTY", path_to_cstring(tty)?)?;
    if let Some(histfile) = histfile {
        params.set("HISTFILE", path_to_cstring(histfile)?)?;
    }

    let profile_text = CString::new(profile_text)
        .map_err(|_| Error::Usage("sandbox profile contains NUL bytes".into()))?;
    let compiled = params.compile(&profile_text)?;
    let rc = unsafe { sandbox_apply(compiled.0) };
    if rc != 0 {
        return Err(Error::io("sandbox_apply", std::io::Error::last_os_error()));
    }
    Ok(())
}

struct Params {
    raw: *mut c_void,
    storage: Vec<CString>,
}

impl Params {
    fn new() -> Result<Self> {
        let raw = unsafe { sandbox_create_params() };
        if raw.is_null() {
            return Err(Error::io(
                "sandbox_create_params",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(Self {
            raw,
            storage: Vec::new(),
        })
    }

    fn set(&mut self, key: &str, value: CString) -> Result<()> {
        let key =
            CString::new(key).map_err(|_| Error::Usage("sandbox key contains NUL bytes".into()))?;
        self.storage.push(key);
        self.storage.push(value);
        let key = self.storage[self.storage.len() - 2].as_c_str();
        let value = self.storage[self.storage.len() - 1].as_c_str();
        let rc = unsafe { sandbox_set_param(self.raw, key.as_ptr(), value.as_ptr()) };
        if rc != 0 {
            return Err(Error::io(
                "sandbox_set_param",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    fn compile(&self, profile: &CStr) -> Result<Profile> {
        let mut errorbuf = std::ptr::null_mut();
        let compiled = unsafe { sandbox_compile_string(profile.as_ptr(), self.raw, &mut errorbuf) };
        if compiled.is_null() {
            let detail = if errorbuf.is_null() {
                "sandbox profile compilation failed".to_string()
            } else {
                unsafe { CStr::from_ptr(errorbuf) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(Error::Sandbox(detail));
        }
        Ok(Profile(compiled))
    }
}

impl Drop for Params {
    fn drop(&mut self) {
        unsafe {
            sandbox_free_params(self.raw);
        }
    }
}

struct Profile(*mut c_void);

impl Drop for Profile {
    fn drop(&mut self) {
        unsafe {
            sandbox_free_profile(self.0);
        }
    }
}
