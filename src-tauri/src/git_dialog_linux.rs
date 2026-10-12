//! GTK dialogs share Tauri's GTK main thread and never use a WebView.
use gtk::prelude::*;
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;

fn dialog(title: &str, description: &str) -> Result<gtk::Dialog> {
    if !gtk::is_initialized_main_thread() {
        return Err("Native token dialog requires the GTK main thread".into());
    }
    let parent = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.is_visible() && window.is_sensitive());
    let dialog = gtk::Dialog::builder()
        .title(title)
        .modal(true)
        .default_width(480)
        .build();
    dialog.set_transient_for(parent.as_ref());
    dialog.add_button("キャンセル", gtk::ResponseType::Cancel);
    dialog.add_button("実行", gtk::ResponseType::Ok);
    dialog.set_default_response(gtk::ResponseType::Ok);
    let area = dialog.content_area();
    area.set_spacing(12);
    area.set_margin_top(16);
    area.set_margin_bottom(16);
    area.set_margin_start(16);
    area.set_margin_end(16);
    let label = gtk::Label::new(Some(description));
    label.set_line_wrap(true);
    label.set_max_width_chars(70);
    label.set_xalign(0.0);
    area.pack_start(&label, false, false, 0);
    Ok(dialog)
}

pub fn choose_token(description: String, names: Vec<String>, preferred: usize) -> Result<usize> {
    if names.is_empty() {
        return Err("No saved tokens".into());
    }
    let dialog = dialog("Git 通信の確認", &description)?;
    let picker = gtk::ComboBoxText::new();
    for name in &names {
        picker.append_text(name);
    }
    picker.set_active(Some(preferred.min(names.len() - 1) as u32));
    dialog.content_area().pack_start(&picker, false, false, 0);
    dialog.show_all();
    let accepted = dialog.run() == gtk::ResponseType::Ok;
    let selected = picker.active().map(|index| index as usize);
    // No widget callbacks or child access remain after this point.
    unsafe {
        dialog.destroy();
    }
    if !accepted {
        return Err("Cancelled".into());
    }
    selected
        .filter(|index| *index < names.len())
        .ok_or("Choose a token".into())
}

pub fn secret(
    title: String,
    description: String,
    placeholder: String,
    creating: bool,
) -> Result<Zeroizing<String>> {
    let dialog = dialog(&title, &description)?;
    let password = gtk::Entry::new();
    password.set_visibility(false);
    password.set_input_purpose(gtk::InputPurpose::Password);
    password.set_max_length(8192);
    password.set_placeholder_text(Some(&placeholder));
    password.set_activates_default(true);
    dialog.content_area().pack_start(&password, false, false, 0);
    let repeated = gtk::Entry::new();
    if creating {
        repeated.set_visibility(false);
        repeated.set_input_purpose(gtk::InputPurpose::Password);
        repeated.set_max_length(8192);
        repeated.set_placeholder_text(Some("Repeat passphrase"));
        repeated.set_activates_default(true);
        dialog.content_area().pack_start(&repeated, false, false, 0);
    }
    dialog.show_all();
    password.grab_focus();
    let accepted = dialog.run() == gtk::ResponseType::Ok;
    let secret = Zeroizing::new(password.text().to_string());
    let repeat = Zeroizing::new(repeated.text().to_string());
    password.set_text("");
    repeated.set_text("");
    // No widget callbacks or child access remain after this point.
    unsafe {
        dialog.destroy();
    }
    if !accepted {
        return Err("Cancelled".into());
    }
    if secret.is_empty() {
        return Err("A nonempty secret is required".into());
    }
    if creating && *secret != *repeat {
        return Err("Passphrases do not match".into());
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc, time::Duration};
    fn visible_dialogs() -> Vec<gtk::Dialog> {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Dialog>().ok())
            .filter(|dialog| dialog.is_visible())
            .collect()
    }
    #[test]
    #[ignore = "requires a GTK display; run with xvfb-run and --test-threads=1"]
    fn native_input_selection_cancel_and_cleanup() {
        gtk::init().unwrap();
        let preferred_ok = Rc::new(Cell::new(false));
        let observed = preferred_ok.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(150), move || {
            for dialog in visible_dialogs() {
                for child in dialog.content_area().children() {
                    if let Ok(picker) = child.downcast::<gtk::ComboBoxText>() {
                        observed.set(picker.active() == Some(1));
                        picker.set_active(Some(0));
                    }
                }
                dialog.response(gtk::ResponseType::Ok);
            }
        });
        assert_eq!(
            choose_token(
                "Reviewed push".into(),
                vec!["Personal".into(), "Work".into()],
                1
            )
            .unwrap(),
            0
        );
        assert!(preferred_ok.get());
        assert!(visible_dialogs().is_empty());
        let masked = Rc::new(Cell::new(false));
        let observed = masked.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(150), move || {
            for dialog in visible_dialogs() {
                for child in dialog.content_area().children() {
                    if let Ok(entry) = child.downcast::<gtk::Entry>() {
                        observed.set(!gtk::prelude::EntryExt::is_visible(&entry));
                        entry.set_text("dummy-native-input");
                    }
                }
                dialog.response(gtk::ResponseType::Ok);
            }
        });
        assert_eq!(
            secret("Register".into(), "PAT".into(), "PAT".into(), false)
                .unwrap()
                .as_str(),
            "dummy-native-input"
        );
        assert!(masked.get());
        assert!(visible_dialogs().is_empty());
        gtk::glib::timeout_add_local_once(Duration::from_millis(150), move || {
            for dialog in visible_dialogs() {
                dialog.response(gtk::ResponseType::Cancel);
            }
        });
        assert!(choose_token("Cancel push".into(), vec!["Personal".into()], 0).is_err());
        assert!(visible_dialogs().is_empty());
    }
}
