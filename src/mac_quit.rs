//! Cocoa's Quit action bypasses winit's window-close event. Cancel that immediate
//! termination and ask eframe to close normally, so Rectangle can park its tabs.
use eframe::egui;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};
use std::{io, sync::OnceLock};

static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

pub fn install(ctx: &egui::Context) -> io::Result<()> {
    if CONTEXT.get().is_some() {
        return Ok(());
    }
    // Called by eframe's app creator on the main thread, after winit has created
    // its application delegate. Keep that delegate and all its lifecycle hooks.
    let class = AnyClass::get(c"NSApplication")
        .ok_or_else(|| io::Error::other("macOS application class is unavailable"))?;
    // SAFETY: These are AppKit's documented sharedApplication/delegate getters;
    // objc2 retains their returned objects for the duration of this function.
    let app: Retained<AnyObject> = unsafe { msg_send![class, sharedApplication] };
    let delegate: Option<Retained<AnyObject>> = unsafe { msg_send![&*app, delegate] };
    let delegate =
        delegate.ok_or_else(|| io::Error::other("macOS application delegate is unavailable"))?;
    let selector = sel!(applicationShouldTerminate:);
    if delegate.class().instance_method(selector).is_some() {
        return Err(io::Error::other(
            "macOS application delegate already handles termination",
        ));
    }
    // SAFETY: Both supported Mac architectures are 64-bit. The documented return
    // type is NSUInteger (Q), with receiver (@), selector (:), and NSApplication
    // argument (@). The callback lives for the process lifetime; no methods are
    // replaced and winit retains ownership of the delegate and event loop.
    let added = unsafe {
        let callback: Imp = std::mem::transmute::<
            extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> usize,
            Imp,
        >(should_terminate);
        objc2::ffi::class_addMethod(
            delegate.class() as *const AnyClass as *mut AnyClass,
            selector,
            callback,
            c"Q@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        return Err(io::Error::other(
            "Could not install macOS graceful quit handling",
        ));
    }
    let _ = CONTEXT.set(ctx.clone());
    Ok(())
}

extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: &AnyObject) -> usize {
    CONTEXT.get().map_or(1, request_close)
}

fn request_close(ctx: &egui::Context) -> usize {
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    ctx.request_repaint();
    // NSTerminateCancel returns to the ordinary event loop, allowing asynchronous
    // parking and error recovery. eframe exits after the final confirmed close.
    // NSTerminateLater would instead enter AppKit's modal run-loop mode.
    0
}

#[test]
fn native_quit_queues_a_cancellable_root_close() {
    let ctx = egui::Context::default();
    let mut reply = 1;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        reply = request_close(ui.ctx());
    });
    assert_eq!(reply, 0);
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close))
    );
    output.textures_delta.clear();
}
