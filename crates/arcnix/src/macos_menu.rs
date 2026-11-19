use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

fn submenu(bar: &NSMenu, title: &str, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let title = NSString::from_str(title);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
    let item = NSMenuItem::new(mtm);
    item.setTitle(&title);
    bar.addItem(&item);
    item.setSubmenu(Some(&menu));
    menu
}

fn item(menu: &NSMenu, title: &str, action: Sel, key: &str) -> Retained<NSMenuItem> {
    // SAFETY: Each caller supplies a standard one-argument AppKit action. A nil target lets
    // AppKit find its handler in the focused window's responder chain.
    unsafe {
        menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        )
    }
}

pub fn install() {
    let mtm = MainThreadMarker::new().expect("the desktop event loop runs on the main thread");
    let bar = NSMenu::new(mtm);
    let app = submenu(&bar, "Arcnix", mtm);
    item(
        &app,
        "About Arcnix",
        sel!(orderFrontStandardAboutPanel:),
        "",
    );
    app.addItem(&NSMenuItem::separatorItem(mtm));
    item(&app, "Hide Arcnix", sel!(hide:), "h");
    item(&app, "Hide Others", sel!(hideOtherApplications:), "h")
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    item(&app, "Show All", sel!(unhideAllApplications:), "");
    app.addItem(&NSMenuItem::separatorItem(mtm));
    item(&app, "Quit Arcnix", sel!(terminate:), "q");

    let edit = submenu(&bar, "Edit", mtm);
    item(&edit, "Undo", sel!(undo:), "z");
    item(&edit, "Redo", sel!(redo:), "z")
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    item(&edit, "Cut", sel!(cut:), "x");
    item(&edit, "Copy", sel!(copy:), "c");
    item(&edit, "Paste", sel!(paste:), "v");
    item(&edit, "Select All", sel!(selectAll:), "a");
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}
