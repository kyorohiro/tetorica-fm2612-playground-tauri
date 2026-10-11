//! Secrets are entered in AppKit, never in HTML or IPC arguments.
use tauri::AppHandle;
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;
pub fn on_main<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce() -> T + Send + 'static,
) -> Result<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(action());
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())
}
pub fn folder(app: &AppHandle, title: &str) -> Result<std::path::PathBuf> {
    let title = title.to_owned();
    on_main(app, move || {
        rfd::FileDialog::new().set_title(title).pick_folder()
    })?
    .ok_or("Cancelled".into())
}
pub fn confirm(app: &AppHandle, title: &str, text: &str) -> Result<()> {
    let title = title.to_owned();
    let text = text.to_owned();
    let yes = on_main(app, move || {
        rfd::MessageDialog::new()
            .set_title(title)
            .set_description(text)
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show()
            == rfd::MessageDialogResult::Ok
    })?;
    if yes {
        Ok(())
    } else {
        Err("Cancelled".into())
    }
}
#[cfg(target_os = "macos")]
pub fn passphrase(
    app: &AppHandle,
    title: &str,
    description: &str,
    creating: bool,
) -> Result<Zeroizing<String>> {
    let title = title.to_owned();
    let description = description.to_owned();
    on_main(app, move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSSecureTextField, NSView};
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
        let mtm =
            MainThreadMarker::new().ok_or("Native password dialog must run on the main thread")?;
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&title));
        alert.setInformativeText(&NSString::from_str(&description));
        alert.addButtonWithTitle(&NSString::from_str("Continue"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        let height = if creating { 70.0 } else { 30.0 };
        let view = NSView::new(mtm);
        view.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(320.0, height),
        ));
        let password = NSSecureTextField::new(mtm);
        password.setFrame(NSRect::new(
            NSPoint::new(0.0, height - 26.0),
            NSSize::new(320.0, 26.0),
        ));
        password.setPlaceholderString(Some(&NSString::from_str("Passphrase (required)")));
        view.addSubview(&password);
        let repeated = NSSecureTextField::new(mtm);
        if creating {
            repeated.setFrame(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(320.0, 26.0),
            ));
            repeated.setPlaceholderString(Some(&NSString::from_str("Repeat passphrase")));
            view.addSubview(&repeated);
        }
        alert.setAccessoryView(Some(&view));
        let accepted = alert.runModal() == NSAlertFirstButtonReturn;
        let secret = Zeroizing::new(password.stringValue().to_string());
        let repeat = Zeroizing::new(repeated.stringValue().to_string());
        password.setStringValue(&NSString::from_str(""));
        repeated.setStringValue(&NSString::from_str(""));
        if !accepted {
            return Err("Cancelled".to_owned());
        }
        if secret.is_empty() {
            return Err("A nonempty passphrase is required".to_owned());
        }
        if creating && *secret != *repeat {
            return Err("Passphrases do not match".to_owned());
        }
        Ok(secret)
    })?
}
#[cfg(not(target_os = "macos"))]
pub fn passphrase(_: &AppHandle, _: &str, _: &str, _: bool) -> Result<Zeroizing<String>> {
    Err("Native secure passphrase entry is currently available on macOS only; SSH is disabled on this platform".into())
}
