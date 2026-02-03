//! Terminal window context.

use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::mem;
#[cfg(not(windows))]
use std::os::unix::io::{AsRawFd, RawFd};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use glutin::config::Config as GlutinConfig;
use glutin::display::GetGlDisplay;
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use glutin::platform::x11::X11GlConfigExt;
use log::info;
use serde_json as json;
use winit::event::{Event as WinitEvent, Modifiers, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::WindowId;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::event_loop::{EventLoop as PtyEventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Direction;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::tty;

use crate::cli::{ParsedOptions, WindowOptions};
use crate::clipboard::Clipboard;
use crate::config::UiConfig;
use crate::display::Display;
use crate::display::SizeInfo;
use crate::display::window::Window;
use crate::event::{
    ActionContext, Event, EventProxy, EventType, InlineSearchState, Mouse, SearchState,
    TouchPurpose,
};
#[cfg(unix)]
use crate::logging::LOG_TARGET_IPC_CONFIG;
use crate::message_bar::MessageBuffer;
use crate::scheduler::Scheduler;
use crate::tabs::{Tab, TabGroup};
use crate::{input, renderer};

/// Event context for one individual Alacritty window.
pub struct WindowContext {
    pub message_buffer: MessageBuffer,
    pub display: Display,
    pub dirty: bool,
    event_queue: Vec<WinitEvent<Event>>,

    /// Tab group containing all tabs for this window.
    /// When tabs are enabled, this manages multiple terminals within a single window.
    pub tab_group: Option<TabGroup>,

    /// Single terminal used when custom tabs are disabled, or when only one tab exists.
    terminal: Arc<FairMutex<Term<EventProxy>>>,
    notifier: Notifier,
    #[cfg(not(windows))]
    master_fd: RawFd,
    #[cfg(not(windows))]
    shell_pid: u32,

    cursor_blink_timed_out: bool,
    prev_bell_cmd: Option<Instant>,
    modifiers: Modifiers,
    inline_search_state: InlineSearchState,
    search_state: SearchState,
    mouse: Mouse,
    touch: TouchPurpose,
    occluded: bool,
    preserve_title: bool,
    window_config: ParsedOptions,
    config: Rc<UiConfig>,
    event_proxy: EventProxy,
}

impl WindowContext {
    /// Create initial window context that does bootstrapping the graphics API we're going to use.
    pub fn initial(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let raw_display_handle = event_loop.display_handle().unwrap().as_raw();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Windows has different order of GL platform initialization compared to any other platform;
        // it requires the window first.
        #[cfg(windows)]
        let window = Window::new(event_loop, &config, &identity, &mut options)?;
        #[cfg(windows)]
        let raw_window_handle = Some(window.raw_window_handle());

        #[cfg(not(windows))]
        let raw_window_handle = None;

        let gl_display = renderer::platform::create_gl_display(
            raw_display_handle,
            raw_window_handle,
            config.debug.prefer_egl,
        )?;
        let gl_config = renderer::platform::pick_gl_config(&gl_display, raw_window_handle)?;

        #[cfg(not(windows))]
        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, &gl_config, raw_window_handle)?;

        let display = Display::new(window, gl_context, &config, false)?;

        Self::new(display, config, options, proxy)
    }

    /// Create additional context with the graphics platform other windows are using.
    pub fn additional(
        gl_config: &GlutinConfig,
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
        config_overrides: ParsedOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let gl_display = gl_config.display();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Check if new window will be opened as a tab.
        // This must be done before `Window::new()`, which unsets `window_tabbing_id`.
        #[cfg(target_os = "macos")]
        let tabbed = options.window_tabbing_id.is_some();
        #[cfg(not(target_os = "macos"))]
        let tabbed = false;

        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let raw_window_handle = window.raw_window_handle();
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, gl_config, Some(raw_window_handle))?;

        let display = Display::new(window, gl_context, &config, tabbed)?;

        let mut window_context = Self::new(display, config, options, proxy)?;

        // Set the config overrides at startup.
        //
        // These are already applied to `config`, so no update is necessary.
        window_context.window_config = config_overrides;

        Ok(window_context)
    }

    /// Create a new terminal window context.
    fn new(
        display: Display,
        config: Rc<UiConfig>,
        options: WindowOptions,
        proxy: EventLoopProxy<Event>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut pty_config = config.pty_config();
        options.terminal_options.override_pty_config(&mut pty_config);

        let preserve_title = options.window_identity.title.is_some();

        info!(
            "PTY dimensions: {:?} x {:?}",
            display.size_info.screen_lines(),
            display.size_info.columns()
        );

        let event_proxy = EventProxy::new(proxy, display.window.id());

        // Create the terminal.
        //
        // This object contains all of the state about what's being displayed. It's
        // wrapped in a clonable mutex since both the I/O loop and display need to
        // access it.
        let terminal = Term::new(config.term_options(), &display.size_info, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        // Create the PTY.
        //
        // The PTY forks a process to run the shell on the slave side of the
        // pseudoterminal. A file descriptor for the master side is retained for
        // reading/writing to the shell.
        let pty = tty::new(&pty_config, display.size_info.into(), display.window.id().into())?;

        #[cfg(not(windows))]
        let master_fd = pty.file().as_raw_fd();
        #[cfg(not(windows))]
        let shell_pid = pty.child().id();

        // Create the pseudoterminal I/O loop.
        //
        // PTY I/O is ran on another thread as to not occupy cycles used by the
        // renderer and input processing. Note that access to the terminal state is
        // synchronized since the I/O loop updates the state, and the display
        // consumes it periodically.
        let pty_event_loop = PtyEventLoop::new(
            Arc::clone(&terminal),
            event_proxy.clone(),
            pty,
            pty_config.drain_on_exit,
            config.debug.ref_test,
        )?;

        // The event loop channel allows write requests from the event processor
        // to be sent to the pty loop and ultimately written to the pty.
        let loop_tx = pty_event_loop.channel();

        // Kick off the I/O thread.
        let _io_thread = pty_event_loop.spawn();

        // Start cursor blinking, in case `Focused` isn't sent on startup.
        if config.cursor.style().blinking {
            event_proxy.send_event(TerminalEvent::CursorBlinkingChange.into());
        }

        // Create context for the Alacritty window.
        Ok(WindowContext {
            preserve_title,
            terminal,
            display,
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
            config,
            notifier: Notifier(loop_tx),
            event_proxy,
            tab_group: None,
            cursor_blink_timed_out: Default::default(),
            prev_bell_cmd: Default::default(),
            inline_search_state: Default::default(),
            message_buffer: Default::default(),
            window_config: Default::default(),
            search_state: Default::default(),
            event_queue: Default::default(),
            modifiers: Default::default(),
            occluded: Default::default(),
            mouse: Default::default(),
            touch: Default::default(),
            dirty: Default::default(),
        })
    }

    /// Create a new tab in this window.
    pub fn create_tab(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.config.tabs.enabled {
            return Ok(());
        }

        // Determine if this will be the second tab (when tab bar will first appear).
        let is_first_new_tab = self.tab_group.is_none();

        // Calculate the correct size_info for the new tab.
        // If this is the first new tab, we need to account for the tab bar that will appear.
        let tab_size_info = if is_first_new_tab {
            // Create a size_info with tab bar height included.
            let current = &self.display.size_info;
            SizeInfo::new_with_tab_bar(
                current.width(),
                current.height(),
                current.cell_width(),
                current.cell_height(),
                current.padding_x(),
                current.padding_y(),
                false,
                current.cell_height() + 8.0, // tab_bar_height = cell + padding
            )
        } else {
            self.display.size_info
        };

        // Determine tab_id for the new tab.
        let new_tab_id = self.tab_group.as_ref().map_or(1, |tg| tg.tabs().len());

        // Create EventProxy with tab_id for proper title routing.
        let tab_event_proxy = EventProxy::with_tab_id(
            self.event_proxy.event_loop_proxy().clone(),
            self.display.window.id(),
            new_tab_id,
        );

        let tab = Tab::new(&self.config, &tab_size_info, tab_event_proxy, new_tab_id)?;

        match &mut self.tab_group {
            Some(tab_group) => {
                tab_group.add_tab(tab);
            },
            None => {
                // First time creating a tab - migrate the existing terminal to a TabGroup
                // Get current title from terminal for the initial tab.
                let initial_title = self.terminal.lock()
                    .title()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| self.config.window.identity.title.clone());
                let initial_tab = Tab {
                    terminal: Arc::clone(&self.terminal),
                    notifier: Notifier(self.notifier.0.clone()),
                    title: initial_title,
                    #[cfg(not(windows))]
                    master_fd: self.master_fd,
                    #[cfg(not(windows))]
                    shell_pid: self.shell_pid,
                };
                let mut tab_group = TabGroup::new(initial_tab);
                tab_group.add_tab(tab);
                self.tab_group = Some(tab_group);

                // Resize the first tab's terminal to account for the new tab bar.
                // The first tab's terminal currently has the old size without tab bar.
                self.terminal.lock().resize(tab_size_info);
                let _ = self.notifier.0.send(Msg::Resize(tab_size_info.into()));
            },
        }

        // Update tab bar visibility - show tab bar when we have 2+ tabs.
        let tab_count = self.tab_group.as_ref().map_or(1, |tg| tg.tabs().len());
        self.display.set_tab_bar_visible(tab_count > 1);

        // Mark entire display as damaged to force full redraw.
        self.display.damage_tracker.frame().mark_fully_damaged();

        self.dirty = true;
        Ok(())
    }

    /// Close the current tab. Returns true if the window should close.
    pub fn close_tab(&mut self) -> bool {
        if let Some(tab_group) = &mut self.tab_group {
            if !tab_group.close_active_tab() {
                // Last tab - window should close
                return true;
            }

            // Update tab bar visibility - hide tab bar when we have 1 or fewer tabs.
            let tab_count = tab_group.tabs().len();
            self.display.set_tab_bar_visible(tab_count > 1);

            // Mark entire display as damaged to force full redraw.
            self.display.damage_tracker.frame().mark_fully_damaged();

            self.dirty = true;
            false
        } else {
            // No tabs - window should close
            true
        }
    }

    /// Select the next tab.
    pub fn select_next_tab(&mut self) {
        if let Some(tab_group) = &mut self.tab_group {
            tab_group.select_next_tab();
            // Mark entire display as damaged to force full redraw when switching tabs.
            self.display.damage_tracker.frame().mark_fully_damaged();
            self.dirty = true;
        }
        // Update window title to match active tab.
        self.sync_window_title_to_active_tab();
    }

    /// Select the previous tab.
    pub fn select_previous_tab(&mut self) {
        if let Some(tab_group) = &mut self.tab_group {
            tab_group.select_previous_tab();
            // Mark entire display as damaged to force full redraw when switching tabs.
            self.display.damage_tracker.frame().mark_fully_damaged();
            self.dirty = true;
        }
        // Update window title to match active tab.
        self.sync_window_title_to_active_tab();
    }

    /// Select a tab by index (0-based).
    pub fn select_tab(&mut self, index: usize) {
        if let Some(tab_group) = &mut self.tab_group {
            tab_group.select_tab(index);
            // Mark entire display as damaged to force full redraw when switching tabs.
            self.display.damage_tracker.frame().mark_fully_damaged();
            self.dirty = true;
        }
        // Update window title to match active tab.
        self.sync_window_title_to_active_tab();
    }

    /// Select the last tab.
    pub fn select_last_tab(&mut self) {
        if let Some(tab_group) = &mut self.tab_group {
            tab_group.select_last_tab();
            // Mark entire display as damaged to force full redraw when switching tabs.
            self.display.damage_tracker.frame().mark_fully_damaged();
            self.dirty = true;
        }
        // Update window title to match active tab.
        self.sync_window_title_to_active_tab();
    }

    /// Update window title to match the active tab's title.
    pub fn sync_window_title_to_active_tab(&mut self) {
        if let Some(tab_group) = &self.tab_group {
            let title = tab_group.active_tab().title.clone();
            self.display.window.set_title(title);
        }
    }

    /// Update the terminal window to the latest config.
    pub fn update_config(&mut self, new_config: Rc<UiConfig>) {
        let old_config = mem::replace(&mut self.config, new_config);

        // Apply ipc config if there are overrides.
        self.config = self.window_config.override_config_rc(self.config.clone());

        self.display.update_config(&self.config);
        self.terminal.lock().set_options(self.config.term_options());

        // Reload cursor if its thickness has changed.
        if (old_config.cursor.thickness() - self.config.cursor.thickness()).abs() > f32::EPSILON {
            self.display.pending_update.set_cursor_dirty();
        }

        if old_config.font != self.config.font {
            let scale_factor = self.display.window.scale_factor;
            let old_resolved = old_config.font.resolve_for_scale(scale_factor);
            let new_resolved = self.config.font.resolve_for_scale(scale_factor);
            // Do not update font size if it has been changed at runtime.
            if self.display.font_size == old_resolved.size().scale(scale_factor as f32) {
                self.display.font_size = new_resolved.size().scale(scale_factor as f32);
            }

            let font = new_resolved.with_size(self.display.font_size);
            self.display.pending_update.set_font(font);
        }

        // Always reload the theme to account for auto-theme switching.
        self.display.window.set_theme(self.config.window.theme());

        // Update display if either padding options or resize increments were changed.
        let window_config = &old_config.window;
        if window_config.padding(1.) != self.config.window.padding(1.)
            || window_config.dynamic_padding != self.config.window.dynamic_padding
            || window_config.resize_increments != self.config.window.resize_increments
        {
            self.display.pending_update.dirty = true;
        }

        // Update title on config reload according to the following table.
        //
        // │cli │ dynamic_title │ current_title == old_config ││ set_title │
        // │ Y  │       _       │              _              ││     N     │
        // │ N  │       Y       │              Y              ││     Y     │
        // │ N  │       Y       │              N              ││     N     │
        // │ N  │       N       │              _              ││     Y     │
        if !self.preserve_title
            && (!self.config.window.dynamic_title
                || self.display.window.title() == old_config.window.identity.title)
        {
            self.display.window.set_title(self.config.window.identity.title.clone());
        }

        let opaque = self.config.window_opacity() >= 1.;

        // Disable shadows for transparent windows on macOS.
        #[cfg(target_os = "macos")]
        self.display.window.set_has_shadow(opaque);

        #[cfg(target_os = "macos")]
        self.display.window.set_option_as_alt(self.config.window.option_as_alt());

        // Change opacity and blur state.
        self.display.window.set_transparent(!opaque);
        self.display.window.set_blur(self.config.window.blur);

        // Update hint keys.
        self.display.hint_state.update_alphabet(self.config.hints.alphabet());

        // Update cursor blinking.
        let event = Event::new(TerminalEvent::CursorBlinkingChange.into(), None);
        self.event_queue.push(event.into());

        self.dirty = true;
    }

    /// Get reference to the window's configuration.
    pub fn config(&self) -> &UiConfig {
        &self.config
    }

    /// Clear the window config overrides.
    #[cfg(unix)]
    pub fn reset_window_config(&mut self, config: Rc<UiConfig>) {
        // Clear previous window errors.
        self.message_buffer.remove_target(LOG_TARGET_IPC_CONFIG);

        self.window_config.clear();

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Add new window config overrides.
    #[cfg(unix)]
    pub fn add_window_config(&mut self, config: Rc<UiConfig>, options: &ParsedOptions) {
        // Clear previous window errors.
        self.message_buffer.remove_target(LOG_TARGET_IPC_CONFIG);

        self.window_config.extend_from_slice(options);

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Draw the window.
    pub fn draw(&mut self, scheduler: &mut Scheduler) {
        self.display.window.requested_redraw = false;

        if self.occluded {
            return;
        }

        self.dirty = false;

        // Force the display to process any pending display update.
        self.display.process_renderer_update();

        // Request immediate re-draw if visual bell animation is not finished yet.
        if !self.display.visual_bell.completed() {
            // We can get an OS redraw which bypasses alacritty's frame throttling, thus
            // marking the window as dirty when we don't have frame yet.
            if self.display.window.has_frame {
                self.display.window.request_redraw();
            } else {
                self.dirty = true;
            }
        }

        // Get the active terminal (from tab group if present, otherwise the main terminal).
        let active_terminal = if let Some(tg) = &self.tab_group {
            Arc::clone(&tg.active_tab().terminal)
        } else {
            Arc::clone(&self.terminal)
        };

        // Redraw the window.
        let terminal = active_terminal.lock();

        // Draw tab bar only if tabs are enabled AND there are 2+ tabs.
        let tab_info = if self.config.tabs.enabled {
            if let Some(tg) = &self.tab_group {
                // Only show tab bar when there are multiple tabs.
                if tg.tabs().len() > 1 {
                    let titles: Vec<String> =
                        tg.tabs().iter().map(|t| t.title.clone()).collect();
                    Some((titles, tg.active_index()))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        self.display.draw(
            terminal,
            scheduler,
            &self.message_buffer,
            &self.config,
            &mut self.search_state,
            tab_info,
        );
    }

    /// Process events for this terminal window.
    pub fn handle_event(
        &mut self,
        #[cfg(target_os = "macos")] event_loop: &ActiveEventLoop,
        event_proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
        scheduler: &mut Scheduler,
        event: WinitEvent<Event>,
    ) {
        // Handle tab events directly before queueing.
        if let WinitEvent::UserEvent(Event { payload, .. }) = &event {
            match payload {
                EventType::CreateTab => {
                    if let Err(err) = self.create_tab() {
                        log::error!("Failed to create tab: {err}");
                    }
                    return;
                },
                EventType::CloseTab => {
                    if self.close_tab() {
                        // Last tab closed - exit the terminal to close the window
                        self.terminal.lock().exit();
                    }
                    return;
                },
                EventType::SelectNextTab => {
                    self.select_next_tab();
                    return;
                },
                EventType::SelectPreviousTab => {
                    self.select_previous_tab();
                    return;
                },
                EventType::SelectTab(index) => {
                    self.select_tab(*index);
                    return;
                },
                EventType::SelectLastTab => {
                    self.select_last_tab();
                    return;
                },
                _ => {},
            }
        }

        match event {
            WinitEvent::AboutToWait
            | WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                // Skip further event handling with no staged updates.
                if self.event_queue.is_empty() {
                    return;
                }

                // Continue to process all pending events.
            },
            event => {
                self.event_queue.push(event);
                return;
            },
        }

        // Get tab count before mutable borrows (for tab bar click detection).
        let tab_count = self.tab_group.as_ref().map_or(0, |tg| tg.tabs().len());

        // Get the active terminal and notifier (from tab group if present).
        #[cfg(not(windows))]
        let (active_terminal_arc, active_notifier, active_master_fd, active_shell_pid) =
            if let Some(tg) = &mut self.tab_group {
                let tab = tg.active_tab_mut();
                (
                    Arc::clone(&tab.terminal),
                    &mut tab.notifier,
                    tab.master_fd,
                    tab.shell_pid,
                )
            } else {
                (
                    Arc::clone(&self.terminal),
                    &mut self.notifier,
                    self.master_fd,
                    self.shell_pid,
                )
            };

        #[cfg(windows)]
        let (active_terminal_arc, active_notifier) =
            if let Some(tg) = &mut self.tab_group {
                let tab = tg.active_tab_mut();
                (Arc::clone(&tab.terminal), &mut tab.notifier)
            } else {
                (Arc::clone(&self.terminal), &mut self.notifier)
            };

        let mut terminal = active_terminal_arc.lock();

        let old_is_searching = self.search_state.history_index.is_some();

        let context = ActionContext {
            cursor_blink_timed_out: &mut self.cursor_blink_timed_out,
            prev_bell_cmd: &mut self.prev_bell_cmd,
            message_buffer: &mut self.message_buffer,
            inline_search_state: &mut self.inline_search_state,
            search_state: &mut self.search_state,
            modifiers: &mut self.modifiers,
            notifier: active_notifier,
            display: &mut self.display,
            mouse: &mut self.mouse,
            touch: &mut self.touch,
            dirty: &mut self.dirty,
            occluded: &mut self.occluded,
            terminal: &mut terminal,
            #[cfg(not(windows))]
            master_fd: active_master_fd,
            #[cfg(not(windows))]
            shell_pid: active_shell_pid,
            preserve_title: self.preserve_title,
            config: &self.config,
            event_proxy,
            #[cfg(target_os = "macos")]
            event_loop,
            clipboard,
            scheduler,
            tab_count,
        };
        // Scope the processor to release borrows before submit_display_update.
        {
            let mut processor = input::Processor::new(context);

            for event in self.event_queue.drain(..) {
                processor.handle_event(event);
            }
        }

        // Process DisplayUpdate events.
        if self.display.pending_update.dirty {
            // Get the active notifier again (fresh borrow after processor scope ends).
            let notifier = if let Some(tg) = &mut self.tab_group {
                &mut tg.active_tab_mut().notifier
            } else {
                &mut self.notifier
            };

            Self::submit_display_update(
                &mut terminal,
                &mut self.display,
                notifier,
                &self.message_buffer,
                &mut self.search_state,
                old_is_searching,
                &self.config,
            );
            self.dirty = true;
        }

        if self.dirty || self.mouse.hint_highlight_dirty {
            self.dirty |= self.display.update_highlighted_hints(
                &terminal,
                &self.config,
                &self.mouse,
                self.modifiers.state(),
            );
            self.mouse.hint_highlight_dirty = false;
        }

        // Don't call `request_redraw` when event is `RedrawRequested` since the `dirty` flag
        // represents the current frame, but redraw is for the next frame.
        if self.dirty
            && self.display.window.has_frame
            && !self.occluded
            && !matches!(event, WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. })
        {
            self.display.window.request_redraw();
        }
    }

    /// ID of this terminal context.
    pub fn id(&self) -> WindowId {
        self.display.window.id()
    }

    /// Write the ref test results to the disk.
    pub fn write_ref_test_results(&self) {
        // Dump grid state.
        let mut grid = self.terminal.lock().grid().clone();
        grid.initialize_all();
        grid.truncate();

        let serialized_grid = json::to_string(&grid).expect("serialize grid");

        let size_info = &self.display.size_info;
        let size = TermSize::new(size_info.columns(), size_info.screen_lines());
        let serialized_size = json::to_string(&size).expect("serialize size");

        let serialized_config = format!("{{\"history_size\":{}}}", grid.history_size());

        File::create("./grid.json")
            .and_then(|mut f| f.write_all(serialized_grid.as_bytes()))
            .expect("write grid.json");

        File::create("./size.json")
            .and_then(|mut f| f.write_all(serialized_size.as_bytes()))
            .expect("write size.json");

        File::create("./config.json")
            .and_then(|mut f| f.write_all(serialized_config.as_bytes()))
            .expect("write config.json");
    }

    /// Submit the pending changes to the `Display`.
    fn submit_display_update(
        terminal: &mut Term<EventProxy>,
        display: &mut Display,
        notifier: &mut Notifier,
        message_buffer: &MessageBuffer,
        search_state: &mut SearchState,
        old_is_searching: bool,
        config: &UiConfig,
    ) {
        // Compute cursor positions before resize.
        let num_lines = terminal.screen_lines();
        let cursor_at_bottom = terminal.grid().cursor.point.line + 1 == num_lines;
        let origin_at_bottom = if terminal.mode().contains(TermMode::VI) {
            terminal.vi_mode_cursor.point.line == num_lines - 1
        } else {
            search_state.direction == Direction::Left
        };

        display.handle_update(terminal, notifier, message_buffer, search_state, config);

        let new_is_searching = search_state.history_index.is_some();
        if !old_is_searching && new_is_searching {
            // Scroll on search start to make sure origin is visible with minimal viewport motion.
            let display_offset = terminal.grid().display_offset();
            if display_offset == 0 && cursor_at_bottom && !origin_at_bottom {
                terminal.scroll_display(Scroll::Delta(1));
            } else if display_offset != 0 && origin_at_bottom {
                terminal.scroll_display(Scroll::Delta(-1));
            }
        }
    }
}

impl Drop for WindowContext {
    fn drop(&mut self) {
        // Tab group handles its own cleanup via Tab::drop.
        if self.tab_group.is_some() {
            return;
        }

        // Shutdown the terminal's PTY for non-tabbed mode.
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}
