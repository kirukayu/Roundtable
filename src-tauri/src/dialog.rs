//! Native file and folder pickers.
//!
//! The browser deliberately refuses to tell a page where a folder lives on disk,
//! which is exactly what this launcher needs to know. So the shell keeps a
//! Windows dialog available and the web interface asks for one over HTTP.
//!
//! These call the Win32 common dialogs directly rather than going through Tauri,
//! because the request arrives on a server thread with no window attached.

use std::path::PathBuf;

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows_sys::core::{GUID, HRESULT, PCWSTR};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };

    // The shell's IFileOpenDialog. Declared by hand so the whole `windows`
    // crate does not have to be pulled in for two dialogs.
    const CLSID_FILE_OPEN_DIALOG: GUID = GUID {
        data1: 0xDC1C5A9C,
        data2: 0xE88A,
        data3: 0x4DDE,
        data4: [0xA5, 0xA1, 0x60, 0xF8, 0x2A, 0x20, 0xAE, 0xF7],
    };

    const IID_IFILE_OPEN_DIALOG: GUID = GUID {
        data1: 0xD57C7288,
        data2: 0xD4AD,
        data3: 0x4768,
        data4: [0xBE, 0x02, 0x9D, 0x96, 0x95, 0x32, 0xD9, 0x60],
    };

    const FOS_PICKFOLDERS: u32 = 0x0000_0020;
    const FOS_FORCEFILESYSTEM: u32 = 0x0000_0040;
    const FOS_FILEMUSTEXIST: u32 = 0x0000_1000;
    const SIGDN_FILESYSPATH: i32 = 0x8005_8000u32 as i32;

    #[repr(C)]
    struct IFileOpenDialogVtbl {
        query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
        release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
        show: unsafe extern "system" fn(*mut core::ffi::c_void, HWND) -> HRESULT,
        set_file_types: unsafe extern "system" fn(*mut core::ffi::c_void, u32, *const FilterSpec) -> HRESULT,
        set_file_type_index: unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> HRESULT,
        get_file_type_index: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32) -> HRESULT,
        advise: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void, *mut u32) -> HRESULT,
        unadvise: unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> HRESULT,
        set_options: unsafe extern "system" fn(*mut core::ffi::c_void, u32) -> HRESULT,
        get_options: unsafe extern "system" fn(*mut core::ffi::c_void, *mut u32) -> HRESULT,
        set_default_folder: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) -> HRESULT,
        set_folder: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) -> HRESULT,
        get_folder: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
        get_current_selection: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
        set_file_name: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> HRESULT,
        get_file_name: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut u16) -> HRESULT,
        set_title: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> HRESULT,
        set_ok_button_label: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> HRESULT,
        set_file_name_label: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> HRESULT,
        get_result: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
    }

    #[repr(C)]
    struct FilterSpec {
        name: PCWSTR,
        spec: PCWSTR,
    }

    #[repr(C)]
    struct IShellItemVtbl {
        query_interface: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
        release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
        bind_to_handler: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void, *const GUID, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
        get_parent: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
        get_display_name: unsafe extern "system" fn(*mut core::ffi::c_void, i32, *mut *mut u16) -> HRESULT,
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Shows the dialog and returns whatever the user picked.
    ///
    /// `folders` switches between the folder and the file variant; `filters` is
    /// a list of bare extensions and only applies to files.
    pub fn show(title: &str, folders: bool, filters: &[String]) -> Option<PathBuf> {
        unsafe {
            // The dialog needs an STA, and this runs on a server thread that has
            // never initialised COM.
            let init = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);

            let mut dialog: *mut core::ffi::c_void = std::ptr::null_mut();
            let hr = CoCreateInstance(
                &CLSID_FILE_OPEN_DIALOG,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IFILE_OPEN_DIALOG,
                &mut dialog,
            );
            if hr < 0 || dialog.is_null() {
                if init >= 0 {
                    CoUninitialize();
                }
                return None;
            }

            let vtbl = *(dialog as *mut *mut IFileOpenDialogVtbl);

            let mut options: u32 = 0;
            ((*vtbl).get_options)(dialog, &mut options);
            options |= FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST;
            if folders {
                options |= FOS_PICKFOLDERS;
            }
            ((*vtbl).set_options)(dialog, options);

            let title_w = wide(title);
            ((*vtbl).set_title)(dialog, title_w.as_ptr());

            // Filter strings have to outlive the call, so they are kept alive
            // here rather than built inline.
            let mut names: Vec<Vec<u16>> = Vec::new();
            let mut specs: Vec<Vec<u16>> = Vec::new();
            let mut filter_specs: Vec<FilterSpec> = Vec::new();
            if !folders && !filters.is_empty() {
                let joined = filters
                    .iter()
                    .map(|extension| format!("*.{extension}"))
                    .collect::<Vec<_>>()
                    .join(";");
                names.push(wide("Supported files"));
                specs.push(wide(&joined));
                names.push(wide("All files"));
                specs.push(wide("*.*"));
                for index in 0..names.len() {
                    filter_specs.push(FilterSpec {
                        name: names[index].as_ptr(),
                        spec: specs[index].as_ptr(),
                    });
                }
                ((*vtbl).set_file_types)(
                    dialog,
                    filter_specs.len() as u32,
                    filter_specs.as_ptr(),
                );
            }

            // A null owner puts the dialog on top of whatever is focused, which
            // is the browser the user is actually looking at.
            let shown = ((*vtbl).show)(dialog, std::ptr::null_mut());

            let mut picked: Option<PathBuf> = None;
            if shown >= 0 {
                let mut item: *mut core::ffi::c_void = std::ptr::null_mut();
                if ((*vtbl).get_result)(dialog, &mut item) >= 0 && !item.is_null() {
                    let item_vtbl = *(item as *mut *mut IShellItemVtbl);
                    let mut raw: *mut u16 = std::ptr::null_mut();
                    if ((*item_vtbl).get_display_name)(item, SIGDN_FILESYSPATH, &mut raw) >= 0
                        && !raw.is_null()
                    {
                        let mut length = 0usize;
                        while *raw.add(length) != 0 {
                            length += 1;
                        }
                        let slice = std::slice::from_raw_parts(raw, length);
                        picked = Some(PathBuf::from(String::from_utf16_lossy(slice)));
                        CoTaskMemFree(raw.cast());
                    }
                    ((*item_vtbl).release)(item);
                }
            }

            ((*vtbl).release)(dialog);
            if init >= 0 {
                CoUninitialize();
            }
            picked
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;
    pub fn show(_title: &str, _folders: bool, _filters: &[String]) -> Option<PathBuf> {
        None
    }
}

pub fn pick_folder(title: &str) -> Option<PathBuf> {
    imp::show(title, true, &[])
}

pub fn pick_file(title: &str, filters: &[String]) -> Option<PathBuf> {
    imp::show(title, false, filters)
}
