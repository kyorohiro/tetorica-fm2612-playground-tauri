//! Win32 modal dialogs with a password edit or a token combo box.
use std::{
    cell::Cell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT},
    UI::{
        Controls::EM_SETLIMITTEXT,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{GetActiveWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;
const FIELD: i32 = 101;
const REPEATED: i32 = 102;
enum Mode {
    Secret {
        placeholder: String,
        creating: bool,
    },
    Choose {
        names: Vec<String>,
        preferred: usize,
    },
}
enum Value {
    Secret(Zeroizing<String>),
    Choice(usize),
}
struct Context {
    title: String,
    description: String,
    mode: Mode,
    result: Option<Result<Value>>,
    cursor: Box<VisibleCursor>,
}
// SetCursor changes the shape, but a negative ShowCursor count still hides it.
// Balance only our own increments when the modal dialog finishes.
struct VisibleCursor {
    increments: Cell<u32>,
    previous: HCURSOR,
}
impl VisibleCursor {
    unsafe fn new() -> Result<Self> {
        let cursor = Self {
            increments: Cell::new(0),
            previous: GetCursor(),
        };
        cursor.ensure_visible()?;
        SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
        Ok(cursor)
    }
    unsafe fn ensure_visible(&self) -> Result<()> {
        for _ in 0..128 {
            let count = ShowCursor(1);
            if count > 0 {
                ShowCursor(0); // Already visible; leave its count unchanged.
                return Ok(());
            }
            self.increments.set(self.increments.get() + 1);
            if count == 0 {
                return Ok(());
            }
        }
        Err("Could not restore cursor visibility for the token dialog".into())
    }
}
impl Drop for VisibleCursor {
    fn drop(&mut self) {
        unsafe {
            for _ in 0..self.increments.get() {
                ShowCursor(0);
            }
            SetCursor(if self.previous.is_null() {
                LoadCursorW(null_mut(), IDC_ARROW)
            } else {
                self.previous
            });
        }
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

// DLGTEMPLATE must be DWORD-aligned; it has no embedded controls. Win32 creates
// the modal owner relationship and handles Tab, Enter, Escape and window close.
fn template() -> Vec<u32> {
    let mut bytes = Vec::new();
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME as u32 | DS_CENTER as u32;
    bytes.extend(style.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    for word in [0u16, 0, 0, 340, 180, 0, 0, 0] {
        bytes.extend(word.to_le_bytes());
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

unsafe fn control(
    hwnd: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> HWND {
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let scale = |value: i32| value * dpi / 96;
    let item = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        scale(x),
        scale(y),
        scale(width),
        scale(height),
        hwnd,
        id as usize as _,
        null_mut(),
        null(),
    );
    if !item.is_null() {
        SendMessageW(
            item,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as usize,
            1,
        );
    }
    item
}
unsafe fn read_secret(hwnd: HWND, id: i32) -> Result<Zeroizing<String>> {
    let field = GetDlgItem(hwnd, id);
    let length = GetWindowTextLengthW(field);
    if length <= 0 || length > 8192 {
        return Err("A nonempty secret of at most 8192 characters is required".into());
    }
    let mut buffer = Zeroizing::new(vec![0u16; length as usize + 1]);
    let count = GetWindowTextW(field, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16(&buffer[..count as usize])
        .map(Zeroizing::new)
        .map_err(|_| "Invalid secret text".into())
}
unsafe fn clear(hwnd: HWND) {
    for id in [FIELD, REPEATED] {
        SetDlgItemTextW(hwnd, id, wide("").as_ptr());
    }
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if message == WM_INITDIALOG {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, lparam);
        let context = &mut *(lparam as *mut Context);
        SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
        SetWindowTextW(hwnd, wide(&context.title).as_ptr());
        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            540 * dpi / 96,
            330 * dpi / 96,
            SWP_NOMOVE | SWP_NOZORDER,
        );
        control(
            hwnd,
            "STATIC",
            &context.description,
            0,
            100,
            16,
            12,
            488,
            110,
        );
        let field = match &context.mode {
            Mode::Secret {
                placeholder,
                creating,
            } => {
                control(hwnd, "STATIC", placeholder, 0, 103, 16, 126, 488, 20);
                let field = control(
                    hwnd,
                    "EDIT",
                    "",
                    WS_BORDER | WS_TABSTOP | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32,
                    FIELD,
                    16,
                    148,
                    488,
                    28,
                );
                SendMessageW(field, EM_SETLIMITTEXT, 8192, 0);
                if *creating {
                    let repeated = control(
                        hwnd,
                        "EDIT",
                        "",
                        WS_BORDER | WS_TABSTOP | ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32,
                        REPEATED,
                        16,
                        184,
                        488,
                        28,
                    );
                    SendMessageW(repeated, EM_SETLIMITTEXT, 8192, 0);
                }
                field
            }
            Mode::Choose { names, preferred } => {
                let field = control(
                    hwnd,
                    "COMBOBOX",
                    "",
                    WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
                    FIELD,
                    16,
                    136,
                    488,
                    180,
                );
                for name in names {
                    SendMessageW(field, CB_ADDSTRING, 0, wide(name).as_ptr() as isize);
                }
                SendMessageW(field, CB_SETCURSEL, (*preferred).min(names.len() - 1), 0);
                field
            }
        };
        let ok = control(
            hwnd,
            "BUTTON",
            "Continue",
            WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            IDOK,
            302,
            244,
            96,
            30,
        );
        let cancel = control(
            hwnd,
            "BUTTON",
            "Cancel",
            WS_TABSTOP,
            IDCANCEL,
            408,
            244,
            96,
            30,
        );
        if field.is_null() || ok.is_null() || cancel.is_null() {
            context.result = Some(Err("Could not create the native token dialog".into()));
            EndDialog(hwnd, 0);
        } else {
            SetFocus(field);
            // Focus changes can hide the cursor after the modal guard starts.
            let _ = context.cursor.ensure_visible();
            SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
        }
        return 0;
    }
    let context = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Context;
    if context.is_null() {
        return 0;
    }
    if message == WM_SETCURSOR && (lparam as u32 & 0xffff) == HTCLIENT {
        let target = wparam as HWND;
        let input = matches!((*context).mode, Mode::Secret { .. })
            && (target == GetDlgItem(hwnd, FIELD) || target == GetDlgItem(hwnd, REPEATED));
        let cursor = LoadCursorW(null_mut(), if input { IDC_IBEAM } else { IDC_ARROW });
        if !cursor.is_null() {
            let _ = (*context).cursor.ensure_visible();
            SetCursor(cursor);
            // DLGPROC's return value marks the message as handled. Its actual
            // result goes in DWLP_MSGRESULT, so DefDlgProc returns TRUE and
            // neither the owner nor the default handler can hide the cursor.
            SetWindowLongPtrW(hwnd, DWLP_MSGRESULT as i32, 1);
            return 1;
        }
    }
    if message == WM_CLOSE || (message == WM_COMMAND && (wparam & 0xffff) == IDCANCEL as usize) {
        clear(hwnd);
        EndDialog(hwnd, 0);
        return 1;
    }
    if message == WM_COMMAND && (wparam & 0xffff) == IDOK as usize {
        let context = &mut *context;
        context.result = Some(match &context.mode {
            Mode::Secret { creating, .. } => read_secret(hwnd, FIELD).and_then(|secret| {
                if *creating && *secret != *read_secret(hwnd, REPEATED)? {
                    return Err("Passphrases do not match".into());
                }
                Ok(Value::Secret(secret))
            }),
            Mode::Choose { names, .. } => {
                let index = SendMessageW(GetDlgItem(hwnd, FIELD), CB_GETCURSEL, 0, 0);
                if index < 0 || index as usize >= names.len() {
                    Err("Choose a token".into())
                } else {
                    Ok(Value::Choice(index as usize))
                }
            }
        });
        clear(hwnd);
        EndDialog(hwnd, 1);
        return 1;
    }
    0
}
fn run(mut context: Context) -> Result<Value> {
    let template = template();
    let outcome = unsafe {
        DialogBoxIndirectParamW(
            null_mut(),
            template.as_ptr().cast(),
            GetActiveWindow(),
            Some(procedure),
            &mut context as *mut Context as isize,
        )
    };
    if outcome == -1 {
        return Err("Could not open the native token dialog".into());
    }
    context.result.unwrap_or_else(|| Err("Cancelled".into()))
}
pub fn choose_token(description: String, names: Vec<String>, preferred: usize) -> Result<usize> {
    if names.is_empty() {
        return Err("No saved tokens".into());
    }
    match run(Context {
        title: "Confirm Git connection".into(),
        description,
        mode: Mode::Choose { names, preferred },
        result: None,
        cursor: Box::new(unsafe { VisibleCursor::new()? }),
    })? {
        Value::Choice(index) => Ok(index),
        _ => Err("Unexpected dialog result".into()),
    }
}
pub fn secret(
    title: String,
    description: String,
    placeholder: String,
    creating: bool,
) -> Result<Zeroizing<String>> {
    match run(Context {
        title,
        description,
        mode: Mode::Secret {
            placeholder,
            creating,
        },
        result: None,
        cursor: Box::new(unsafe { VisibleCursor::new()? }),
    })? {
        Value::Secret(secret) => Ok(secret),
        _ => Err("Unexpected dialog result".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    struct Cursor(HCURSOR);
    impl Drop for Cursor {
        fn drop(&mut self) {
            unsafe {
                SetCursor(self.0);
            }
        }
    }
    unsafe fn display_count() -> i32 {
        ShowCursor(1);
        ShowCursor(0)
    }
    #[test]
    fn modal_cursor_is_visible_even_when_the_owner_hid_it_and_restores_the_count() {
        unsafe {
            let _shape = Cursor(GetCursor());
            struct Hidden(u32);
            impl Drop for Hidden {
                fn drop(&mut self) {
                    unsafe {
                        for _ in 0..self.0 {
                            ShowCursor(1);
                        }
                    }
                }
            }
            let initial = display_count();
            {
                let mut hidden = Hidden(0);
                let count = loop {
                    hidden.0 += 1;
                    let count = ShowCursor(0);
                    if count <= -3 {
                        break count;
                    }
                };
                let visible = VisibleCursor::new().unwrap();
                assert_eq!(display_count(), 0);
                assert!(!GetCursor().is_null());
                visible.ensure_visible().unwrap();
                assert_eq!(display_count(), 0);
                drop(visible);
                assert_eq!(display_count(), count);
            }
            assert_eq!(display_count(), initial);
        }
    }
    #[test]
    fn dialog_restores_cursor_on_background_controls_and_password_input() {
        unsafe {
            let _cursor = Cursor(GetCursor());
            for mode in [
                Mode::Choose {
                    names: vec!["Personal".into(), "Work".into()],
                    preferred: 1,
                },
                Mode::Secret {
                    placeholder: "PAT".into(),
                    creating: false,
                },
            ] {
                let secret = matches!(mode, Mode::Secret { .. });
                let mut context = Context {
                    title: "Cursor test".into(),
                    description: "Dummy UI".into(),
                    mode,
                    result: None,
                    cursor: Box::new(VisibleCursor::new().unwrap()),
                };
                let template = template();
                let hwnd = CreateDialogIndirectParamW(
                    null_mut(),
                    template.as_ptr().cast(),
                    null_mut(),
                    Some(procedure),
                    &mut context as *mut Context as isize,
                );
                assert!(!hwnd.is_null());
                let window = Window(hwnd);
                for (target, expected) in [
                    (hwnd, IDC_ARROW),
                    (GetDlgItem(hwnd, IDOK), IDC_ARROW),
                    (
                        GetDlgItem(hwnd, FIELD),
                        if secret { IDC_IBEAM } else { IDC_ARROW },
                    ),
                ] {
                    assert!(!target.is_null());
                    SetCursor(null_mut());
                    assert_eq!(
                        SendMessageW(hwnd, WM_SETCURSOR, target as usize, HTCLIENT as isize),
                        1
                    );
                    assert_eq!(GetCursor(), LoadCursorW(null_mut(), expected));
                }
                drop(window); // The callback context must outlive the window.
            }
        }
    }
}
