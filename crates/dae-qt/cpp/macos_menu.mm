// Standard macOS application menu for the Qt Quick shell.
//
// Qt only creates an NSMenu when a QMenuBar exists. The shell uses QML for all
// of its chrome, so it never creates one, and macOS therefore shows no menu
// bar at all. This file builds the minimum a native app is expected to have:
// the app menu (About/Services/Hide/Quit), File, Edit and Window. Items target
// nil, which sends the action down the responder chain; Qt's NSView already
// implements the editing selectors, so they act on the focused QML text input.

#include "macos_menu.h"

#if defined(Q_OS_MACOS)

#import <AppKit/AppKit.h>

namespace {

NSMenuItem* make_item(NSString* title, SEL action, NSString* key,
                      NSEventModifierFlags flags)
{
    NSMenuItem* item = [[NSMenuItem alloc] initWithTitle:title
                                                  action:action
                                           keyEquivalent:key];
    item.keyEquivalentModifierMask = flags;
    return item;
}

NSMenuItem* submenu_item(NSMenu* parent, NSString* title)
{
    NSMenuItem* item = [[NSMenuItem alloc] initWithTitle:title
                                                  action:nil
                                           keyEquivalent:@""];
    NSMenu* submenu = [[NSMenu alloc] initWithTitle:title];
    item.submenu = submenu;
    [parent addItem:item];
    return item;
}

} // namespace

void dae_qt_install_macos_menu()
{
    @autoreleasepool {
        if (NSApp == nil)
            return;

        NSMenu* mainMenu = [[NSMenu alloc] initWithTitle:@"MainMenu"];
        NSString* appName = [[NSProcessInfo processInfo] processName];

        // Application menu. The title is ignored by macOS; the system draws
        // the process name in bold from the bundle.
        NSMenuItem* appMenuItem = [[NSMenuItem alloc] initWithTitle:@""
                                                             action:nil
                                                      keyEquivalent:@""];
        [mainMenu addItem:appMenuItem];
        NSMenu* appMenu = [[NSMenu alloc] initWithTitle:@""];
        appMenuItem.submenu = appMenu;

        [appMenu addItemWithTitle:[NSString stringWithFormat:@"About %@", appName]
                           action:@selector(orderFrontStandardAboutPanel:)
                    keyEquivalent:@""];
        [appMenu addItem:[NSMenuItem separatorItem]];

        NSMenuItem* servicesItem = submenu_item(appMenu, @"Services");
        NSApp.servicesMenu = servicesItem.submenu;

        [appMenu addItem:[NSMenuItem separatorItem]];
        [appMenu addItem:make_item([NSString stringWithFormat:@"Hide %@", appName],
                                   @selector(hide:), @"h",
                                   NSEventModifierFlagCommand)];
        [appMenu addItem:make_item(@"Hide Others",
                                   @selector(hideOtherApplications:), @"h",
                                   NSEventModifierFlagCommand | NSEventModifierFlagOption)];
        [appMenu addItem:make_item(@"Show All",
                                   @selector(unhideAllApplications:), @"", 0)];
        [appMenu addItem:[NSMenuItem separatorItem]];
        [appMenu addItem:make_item([NSString stringWithFormat:@"Quit %@", appName],
                                   @selector(terminate:), @"q",
                                   NSEventModifierFlagCommand)];

        // Application commands belong to NSApp, not to whichever view happens
        // to be first responder when the menu is clicked.
        for (NSMenuItem* item in appMenu.itemArray) {
            if (item.action != nullptr)
                item.target = NSApp;
        }

        // File menu. Qt's QNSView implements performClose: through the window.
        NSMenuItem* fileItem = submenu_item(mainMenu, @"File");
        [fileItem.submenu addItem:make_item(@"Close Window",
                                            @selector(performClose:), @"w",
                                            NSEventModifierFlagCommand)];

        // Edit menu. These selectors are handled by Qt's NSView through the
        // responder chain, which is what makes them work on QML TextInput.
        NSMenuItem* editItem = submenu_item(mainMenu, @"Edit");
        NSMenu* editMenu = editItem.submenu;
        [editMenu addItem:make_item(@"Undo", @selector(undo:), @"z",
                                    NSEventModifierFlagCommand)];
        [editMenu addItem:make_item(@"Redo", @selector(redo:), @"z",
                                    NSEventModifierFlagCommand | NSEventModifierFlagShift)];
        [editMenu addItem:[NSMenuItem separatorItem]];
        [editMenu addItem:make_item(@"Cut", @selector(cut:), @"x",
                                    NSEventModifierFlagCommand)];
        [editMenu addItem:make_item(@"Copy", @selector(copy:), @"c",
                                    NSEventModifierFlagCommand)];
        [editMenu addItem:make_item(@"Paste", @selector(paste:), @"v",
                                    NSEventModifierFlagCommand)];
        [editMenu addItem:make_item(@"Select All", @selector(selectAll:), @"a",
                                    NSEventModifierFlagCommand)];

        // Window menu. AppKit fills the window list into this menu itself.
        NSMenuItem* windowItem = submenu_item(mainMenu, @"Window");
        NSMenu* windowMenu = windowItem.submenu;
        [windowMenu addItem:make_item(@"Minimize", @selector(performMiniaturize:), @"m",
                                      NSEventModifierFlagCommand)];
        [windowMenu addItem:make_item(@"Zoom", @selector(performZoom:), @"", 0)];
        [windowMenu addItem:[NSMenuItem separatorItem]];
        NSMenuItem* frontItem = make_item(@"Bring All to Front",
                                          @selector(arrangeInFront:), @"", 0);
        frontItem.target = NSApp;
        [windowMenu addItem:frontItem];
        [windowMenu addItem:[NSMenuItem separatorItem]];
        [windowMenu addItem:make_item(@"Enter Full Screen",
                                      @selector(toggleFullScreen:), @"f",
                                      NSEventModifierFlagCommand | NSEventModifierFlagControl)];

        NSApp.mainMenu = mainMenu;
        NSApp.windowsMenu = windowMenu;
    }
}

#endif // Q_OS_MACOS
