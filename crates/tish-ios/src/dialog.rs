//! `dialog.message` / `dialog.confirm` / `dialog.ask` via `UIAlertController`.
//!
//! Nested CFRunLoop *waits* from inside a UIControl action freeze the app.
//! `dialog.message` therefore presents and returns immediately (system dismisses on OK).
//! confirm/ask defer present via a 0-delay `NSTimer` (same thread, no `Send`), then pump.

use serde_json::{json, Value};

#[cfg(target_os = "ios")]
mod inner {
    use std::ptr::NonNull;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicI8, Ordering};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_core_foundation::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
    use objc2_foundation::{NSString, NSTimer};
    use objc2_ui_kit::{
        UIAlertAction, UIAlertActionStyle, UIAlertController, UIAlertControllerStyle,
        UIViewController,
    };
    use serde_json::{json, Value};

    use crate::uikit::host::presenting_view_controller;

    fn title_message(args: &Value) -> (String, String) {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Tish")
            .to_string();
        let message = args
            .get("message")
            .or_else(|| args.get("body"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        (title, message)
    }

    fn pump_main(seconds: f64) {
        let mode = unsafe { kCFRunLoopCommonModes }.or_else(|| unsafe { kCFRunLoopDefaultMode });
        if let Some(mode) = mode {
            let _ = CFRunLoop::run_in_mode(Some(mode), seconds, false);
        }
    }

    fn make_alert(
        mtm: MainThreadMarker,
        title: &str,
        message: &str,
    ) -> Retained<UIAlertController> {
        UIAlertController::alertControllerWithTitle_message_preferredStyle(
            Some(&NSString::from_str(title)),
            Some(&NSString::from_str(message)),
            UIAlertControllerStyle::Alert,
            mtm,
        )
    }

    /// Non-blocking — safe from a button `onClick`.
    fn present_message(title: &str, message: &str) -> Result<(), String> {
        let mtm =
            MainThreadMarker::new().ok_or_else(|| "dialog.* requires main thread".to_string())?;
        let vc = presenting_view_controller()
            .ok_or_else(|| "no presenting view controller (call ios.run first)".to_string())?;

        if vc.presentedViewController().is_some() {
            return Ok(());
        }

        let alert = make_alert(mtm, title, message);
        let ok = UIAlertAction::actionWithTitle_style_handler(
            Some(&NSString::from_str("OK")),
            UIAlertActionStyle::Default,
            None,
            mtm,
        );
        alert.addAction(&ok);
        vc.presentViewController_animated_completion(&alert, true, None);
        Ok(())
    }

    /// Blocking choices: schedule present on the next runloop turn, then pump until tapped.
    fn present_choices(
        title: &str,
        message: &str,
        actions: &[(&str, UIAlertActionStyle, i8)],
    ) -> Result<i8, String> {
        let mtm =
            MainThreadMarker::new().ok_or_else(|| "dialog.* requires main thread".to_string())?;
        let vc = presenting_view_controller()
            .ok_or_else(|| "no presenting view controller (call ios.run first)".to_string())?;

        let result = Rc::new(AtomicI8::new(-1));
        let alert = make_alert(mtm, title, message);

        let mut keep_actions = Vec::new();
        for (label, style, code) in actions {
            let code = *code;
            let result = Rc::clone(&result);
            let handler = RcBlock::new(move |_action: NonNull<UIAlertAction>| {
                result.store(code, Ordering::SeqCst);
            });
            let action = UIAlertAction::actionWithTitle_style_handler(
                Some(&NSString::from_str(label)),
                *style,
                Some(&*handler),
                mtm,
            );
            alert.addAction(&action);
            keep_actions.push(handler);
        }

        // Present on the next turn so we are not mid-UIControl sendActions.
        let presenter: Retained<UIViewController> = vc.clone();
        let present_block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if presenter.presentedViewController().is_none() {
                presenter.presentViewController_animated_completion(&alert, true, None);
            }
        });
        let _timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.0, false, &*present_block)
        };
        // Keep blocks / timer target alive for the duration of the wait.
        let _keep = (keep_actions, present_block, _timer);

        let deadline = Instant::now() + Duration::from_secs(120);
        while result.load(Ordering::SeqCst) < 0 {
            if Instant::now() > deadline {
                if vc.presentedViewController().is_some() {
                    vc.dismissViewControllerAnimated_completion(false, None);
                }
                return Err("dialog timed out".into());
            }
            pump_main(0.05);
        }
        Ok(result.load(Ordering::SeqCst))
    }

    pub fn invoke(cmd: &str, args: &Value) -> Result<Value, String> {
        match cmd {
            "dialog.message" => {
                let (title, message) = title_message(args);
                present_message(&title, &message)?;
                Ok(json!({ "ok": true }))
            }
            "dialog.confirm" => {
                let (title, message) = title_message(args);
                let code = present_choices(
                    &title,
                    &message,
                    &[
                        ("Cancel", UIAlertActionStyle::Cancel, 0),
                        ("OK", UIAlertActionStyle::Default, 1),
                    ],
                )?;
                Ok(json!({ "ok": true, "confirmed": code == 1 }))
            }
            "dialog.ask" => {
                let (title, message) = title_message(args);
                let yes = args
                    .get("okLabel")
                    .or_else(|| args.get("yesLabel"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Yes");
                let no = args
                    .get("cancelLabel")
                    .or_else(|| args.get("noLabel"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("No");
                let code = present_choices(
                    &title,
                    &message,
                    &[
                        (no, UIAlertActionStyle::Cancel, 0),
                        (yes, UIAlertActionStyle::Default, 1),
                    ],
                )?;
                Ok(json!({ "ok": true, "confirmed": code == 1 }))
            }
            _ => Err(format!("unsupported dialog command: {cmd}")),
        }
    }
}

#[cfg(not(target_os = "ios"))]
mod inner {
    use serde_json::{json, Value};

    pub fn invoke(cmd: &str, _args: &Value) -> Result<Value, String> {
        Ok(json!({
            "ok": false,
            "code": "unsupported",
            "capability": "dialog",
            "message": format!("{cmd} only available on iOS"),
        }))
    }
}

pub fn invoke(cmd: &str, args: &Value) -> Result<Value, String> {
    if !cmd.starts_with("dialog.") {
        return Err(format!("not a dialog command: {cmd}"));
    }
    match cmd {
        "dialog.message" | "dialog.confirm" | "dialog.ask" => inner::invoke(cmd, args),
        _ => Ok(json!({
            "ok": false,
            "code": "unsupported",
            "capability": "dialog",
            "message": format!("{cmd} not implemented on iOS"),
        })),
    }
}
