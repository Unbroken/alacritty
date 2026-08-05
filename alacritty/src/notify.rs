#![allow(non_snake_case)]
//! Desktop notification support for bell events.
//!
//! Provides platform-specific notification backends:
//! - macOS: UNUserNotificationCenter via objc2-user-notifications
//! - Linux: D-Bus notifications via notify-rust
//! - Windows: Toast notifications via notify-rust

use std::collections::HashMap;

use log::debug;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::event::Event;

/// Tracks an active (delivered) notification for deduplication.
struct ActiveNotification {
    /// The body text used for deduplication comparison.
    body: String,
    /// Platform-specific notification identifier for removal/replacement.
    platform_id: PlatformNotificationId,
}

// ─── macOS ──────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
type PlatformNotificationId = String;

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use log::debug;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, MainThreadOnly};
    use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use winit::event_loop::EventLoopProxy;
    use winit::window::WindowId;

    use crate::event::{Event, EventType};

    type WindowMap = std::sync::Mutex<HashMap<String, (WindowId, Option<usize>)>>;

    /// Global event loop proxy for the delegate callback.
    static EVENT_PROXY: OnceLock<EventLoopProxy<Event>> = OnceLock::new();

    /// Mapping from notification identifier to (WindowId, tab_id).
    static WINDOW_MAP: OnceLock<WindowMap> = OnceLock::new();

    pub fn window_map() -> &'static WindowMap {
        WINDOW_MAP.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "AlacrittyNotificationDelegate"]
        pub struct NotificationDelegate;

        unsafe impl NSObjectProtocol for NotificationDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn userNotificationCenter_didReceiveNotificationResponse_withCompletionHandler(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion_handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let identifier = response.notification().request().identifier();
                let id_str = identifier.to_string();
                debug!("Notification clicked: {}", id_str);

                if let Some(map) = WINDOW_MAP.get()
                    && let Ok(map) = map.lock()
                    && let Some(&(window_id, tab_id)) = map.get(&id_str)
                    && let Some(proxy) = EVENT_PROXY.get()
                {
                    let event = Event::new(
                        EventType::FocusTab { window_id, tab_id },
                        None,
                    );
                    let _ = proxy.send_event(event);
                }

                completion_handler.call(());
            }

            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn userNotificationCenter_willPresentNotification_withCompletionHandler(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion_handler: &block2::DynBlock<
                    dyn Fn(UNNotificationPresentationOptions),
                >,
            ) {
                // Show banner + sound even when the app is in the foreground.
                let options = UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::Sound;
                completion_handler.call((options,));
            }
        }
    );

    // Raw ObjC runtime FFI to bypass objc2 msg_send! macro trait resolution
    // conflict between objc2 0.5 (winit dep) and 0.6 (direct dep).
    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        #[link_name = "objc_msgSend"]
        fn objc_msg_send(obj: *const std::ffi::c_void, sel: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *const std::ffi::c_void;
    }

    impl NotificationDelegate {
        pub fn new(_mtm: objc2::MainThreadMarker) -> Retained<Self> {
            // define_class! registers the class with the ObjC runtime lazily;
            // class() forces registration. A raw objc_getClass lookup returns
            // null if nothing has touched the class yet.
            let cls = <Self as objc2::ClassType>::class();
            unsafe {
                let alloc_sel = sel_registerName(c"alloc".as_ptr());
                let init_sel = sel_registerName(c"init".as_ptr());

                let obj = objc_msg_send(cls as *const _ as *const std::ffi::c_void, alloc_sel);
                let obj = objc_msg_send(obj, init_sel);

                Retained::from_raw(obj as *mut Self)
                    .expect("NotificationDelegate init failed")
            }
        }
    }

    /// Initialize macOS notification support.
    pub fn init(proxy: EventLoopProxy<Event>) {
        EVENT_PROXY.set(proxy).ok();

        if let Some(mtm) = objc2::MainThreadMarker::new() {
            let center = UNUserNotificationCenter::currentNotificationCenter();

            // Request authorization (non-blocking).
            let completion = block2::RcBlock::new(
                |_granted: objc2::runtime::Bool, _error: *mut objc2_foundation::NSError| {
                    debug!("Notification authorization response received");
                },
            );
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &completion,
            );

            // Set up delegate. Leak to keep alive (center holds a weak ref).
            let delegate = NotificationDelegate::new(mtm);
            let delegate_proto: Retained<ProtocolObject<dyn UNUserNotificationCenterDelegate>> =
                ProtocolObject::from_retained(delegate);
            center.setDelegate(Some(&delegate_proto));
            std::mem::forget(delegate_proto);
        }
    }

    /// Build a notification identifier for a window/tab pair.
    pub fn notification_id(window_id: WindowId, tab_id: Option<usize>) -> String {
        format!("alacritty-bell-{:?}-{}", window_id, tab_id.unwrap_or(0))
    }

    /// Send a macOS notification.
    pub fn send(identifier: &str, title: &str, body: &str) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));

        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(identifier),
            &content,
            None,
        );

        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.addNotificationRequest_withCompletionHandler(&request, None);
    }

    /// Remove a delivered macOS notification by identifier.
    pub fn remove(identifier: &str) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let id_nsstring = NSString::from_str(identifier);
        let identifiers = NSArray::from_retained_slice(&[id_nsstring]);
        center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
    }
}

// ─── Linux ──────────────────────────────────────────────────────────────────

#[cfg(all(unix, not(target_os = "macos")))]
type PlatformNotificationId = u32;

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::thread;

    use log::{debug, warn};
    use notify_rust::Notification;
    use winit::event_loop::EventLoopProxy;
    use winit::window::WindowId;

    use crate::event::{Event, EventType};

    pub fn init(_proxy: EventLoopProxy<Event>) {}

    /// Send a Linux D-Bus notification. Returns the notification ID.
    pub fn send(
        title: &str,
        body: &str,
        replaces_id: Option<u32>,
        proxy: EventLoopProxy<Event>,
        window_id: WindowId,
        tab_id: Option<usize>,
    ) -> Option<u32> {
        let mut notification = Notification::new();
        notification
            .appname("Alacritty")
            .summary(title)
            .body(body)
            .action("default", "default");

        if let Some(id) = replaces_id {
            notification.id(id);
        }

        match notification.show() {
            Ok(handle) => {
                let notification_id = handle.id();
                debug!("Sent D-Bus notification id={}", notification_id);

                // Spawn a thread to wait for the click action.
                thread::spawn(move || {
                    handle.wait_for_action(|action| {
                        if action == "default" {
                            debug!(
                                "Notification clicked: window={:?} tab={:?}",
                                window_id, tab_id
                            );
                            let event = Event::new(
                                EventType::FocusTab { window_id, tab_id },
                                None,
                            );
                            let _ = proxy.send_event(event);
                        }
                    });
                });

                Some(notification_id)
            },
            Err(err) => {
                warn!("Failed to send notification: {}", err);
                None
            },
        }
    }

    /// Close a notification by D-Bus ID using gio.
    pub fn remove(notification_id: u32) {
        use gio::prelude::*;

        debug!("Closing D-Bus notification id={}", notification_id);
        if let Ok(connection) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
            let _ = connection.call_sync(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "CloseNotification",
                Some(&(notification_id,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
                gio::Cancellable::NONE,
            );
        }
    }
}

// ─── Windows ────────────────────────────────────────────────────────────────

#[cfg(windows)]
type PlatformNotificationId = ();

#[cfg(windows)]
mod platform {
    use log::{debug, warn};
    use notify_rust::Notification;
    use winit::event_loop::EventLoopProxy;

    use crate::event::Event;

    pub fn init(_proxy: EventLoopProxy<Event>) {}

    /// Send a Windows Toast notification.
    pub fn send(title: &str, body: &str) {
        match Notification::new().appname("Alacritty").summary(title).body(body).show() {
            Ok(_) => debug!("Sent Windows toast notification"),
            Err(err) => warn!("Failed to send notification: {}", err),
        }
    }
}

// ─── Unified DesktopNotifier ────────────────────────────────────────────────

/// Manages desktop notifications for bell events across all platforms.
pub struct DesktopNotifier {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    proxy: EventLoopProxy<Event>,
    /// Active notifications per (window_id, tab_id) for deduplication.
    active: HashMap<(WindowId, Option<usize>), ActiveNotification>,
    /// Windows-only: pending notification click target.
    #[cfg(windows)]
    pending_click: Option<(std::time::Instant, WindowId, Option<usize>)>,
}

impl DesktopNotifier {
    /// Create a new DesktopNotifier.
    pub fn new(proxy: EventLoopProxy<Event>) -> Self {
        platform::init(proxy.clone());
        DesktopNotifier {
            proxy,
            active: HashMap::new(),
            #[cfg(windows)]
            pending_click: None,
        }
    }

    /// Send or deduplicate a bell notification.
    pub fn notify(
        &mut self,
        window_id: WindowId,
        tab_id: Option<usize>,
        title: &str,
        body: &str,
    ) {
        let key = (window_id, tab_id);

        if let Some(existing) = self.active.get(&key) {
            if existing.body == body {
                debug!("Skipping duplicate notification for {:?}", key);
                return;
            }
            self.remove_platform_notification(&existing.platform_id, window_id, tab_id);
        }

        let platform_id = self.send_platform_notification(window_id, tab_id, title, body);

        self.active.insert(key, ActiveNotification {
            body: body.to_owned(),
            platform_id,
        });
    }

    /// Remove the notification for a specific window/tab.
    pub fn clear(&mut self, window_id: WindowId, tab_id: Option<usize>) {
        let key = (window_id, tab_id);
        if let Some(notification) = self.active.remove(&key) {
            self.remove_platform_notification(&notification.platform_id, window_id, tab_id);
        }
    }

    /// Remove all notifications for a window.
    pub fn clear_all_for_window(&mut self, window_id: WindowId) {
        let keys_to_remove: Vec<_> = self
            .active
            .keys()
            .filter(|(wid, _)| *wid == window_id)
            .copied()
            .collect();

        for key in keys_to_remove {
            if let Some(notification) = self.active.remove(&key) {
                self.remove_platform_notification(&notification.platform_id, key.0, key.1);
            }
        }
    }

    /// Windows only: check if there's a pending notification click target.
    #[cfg(windows)]
    pub fn check_pending_click(&mut self, window_id: WindowId) -> Option<usize> {
        if let Some((instant, wid, tab_id)) = self.pending_click.take() {
            if wid == window_id && instant.elapsed() < std::time::Duration::from_secs(2) {
                return tab_id;
            }
        }
        None
    }

    // ─── macOS dispatch ─────────────────────────────────────────────────

    #[cfg(target_os = "macos")]
    fn send_platform_notification(
        &self,
        window_id: WindowId,
        tab_id: Option<usize>,
        title: &str,
        body: &str,
    ) -> PlatformNotificationId {
        let identifier = platform::notification_id(window_id, tab_id);

        if let Ok(mut map) = platform::window_map().lock() {
            map.insert(identifier.clone(), (window_id, tab_id));
        }

        platform::send(&identifier, title, body);
        identifier
    }

    #[cfg(target_os = "macos")]
    fn remove_platform_notification(
        &self,
        identifier: &PlatformNotificationId,
        _window_id: WindowId,
        _tab_id: Option<usize>,
    ) {
        platform::remove(identifier);
        if let Ok(mut map) = platform::window_map().lock() {
            map.remove(identifier);
        }
    }

    // ─── Linux dispatch ─────────────────────────────────────────────────

    #[cfg(all(unix, not(target_os = "macos")))]
    fn send_platform_notification(
        &mut self,
        window_id: WindowId,
        tab_id: Option<usize>,
        title: &str,
        body: &str,
    ) -> PlatformNotificationId {
        let replaces_id = self
            .active
            .get(&(window_id, tab_id))
            .map(|n| n.platform_id);

        platform::send(title, body, replaces_id, self.proxy.clone(), window_id, tab_id)
            .unwrap_or(0)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn remove_platform_notification(
        &self,
        notification_id: &PlatformNotificationId,
        _window_id: WindowId,
        _tab_id: Option<usize>,
    ) {
        if *notification_id != 0 {
            platform::remove(*notification_id);
        }
    }

    // ─── Windows dispatch ───────────────────────────────────────────────

    #[cfg(windows)]
    fn send_platform_notification(
        &mut self,
        window_id: WindowId,
        tab_id: Option<usize>,
        title: &str,
        body: &str,
    ) -> PlatformNotificationId {
        platform::send(title, body);
        self.pending_click = Some((std::time::Instant::now(), window_id, tab_id));
    }

    #[cfg(windows)]
    fn remove_platform_notification(
        &self,
        _id: &PlatformNotificationId,
        _window_id: WindowId,
        _tab_id: Option<usize>,
    ) {
        // Not supported on Windows via notify-rust.
    }
}
