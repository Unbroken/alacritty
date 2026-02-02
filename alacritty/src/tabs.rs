//! Tab management for terminal windows.
//!
//! This module provides support for multiple terminal tabs within a single window.
//! Each tab contains its own terminal state, PTY, and associated resources.

use std::sync::Arc;

#[cfg(not(windows))]
use std::os::unix::io::RawFd;

use log::info;

use alacritty_terminal::event_loop::{EventLoop as PtyEventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty;

use crate::config::UiConfig;
use crate::display::SizeInfo;
use crate::event::EventProxy;

/// A single terminal tab containing its own terminal state and PTY.
pub struct Tab {
    /// Terminal state.
    pub terminal: Arc<FairMutex<Term<EventProxy>>>,

    /// Notifier for writing to the PTY.
    pub notifier: Notifier,

    /// Tab title (cached from terminal).
    pub title: String,

    /// Master file descriptor for the PTY.
    #[cfg(not(windows))]
    pub master_fd: RawFd,

    /// Shell process ID.
    #[cfg(not(windows))]
    pub shell_pid: u32,
}

impl Tab {
    /// Create a new tab with its own terminal and PTY.
    pub fn new(
        config: &UiConfig,
        size_info: &SizeInfo,
        event_proxy: EventProxy,
        _tab_id: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let pty_config = config.pty_config();

        info!(
            "Creating new tab with PTY dimensions: {:?} x {:?}",
            size_info.screen_lines(),
            size_info.columns()
        );

        // Create the terminal.
        let terminal = Term::new(config.term_options(), size_info, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        // Create the PTY.
        let pty = tty::new(&pty_config, (*size_info).into(), 0)?;

        #[cfg(not(windows))]
        let master_fd = {
            use std::os::unix::io::AsRawFd;
            pty.file().as_raw_fd()
        };
        #[cfg(not(windows))]
        let shell_pid = pty.child().id();

        // Create the PTY event loop.
        let event_loop = PtyEventLoop::new(
            Arc::clone(&terminal),
            event_proxy,
            pty,
            pty_config.drain_on_exit,
            config.debug.ref_test,
        )?;

        let loop_tx = event_loop.channel();

        // Start the I/O thread.
        let _io_thread = event_loop.spawn();

        Ok(Tab {
            terminal,
            notifier: Notifier(loop_tx),
            title: String::from("Terminal"),
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
        })
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        // Shutdown the terminal's PTY.
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

/// A group of tabs within a single window.
pub struct TabGroup {
    /// All tabs in this group.
    tabs: Vec<Tab>,

    /// Index of the currently active tab.
    active: usize,
}

impl TabGroup {
    /// Create a new tab group with an initial tab.
    pub fn new(initial_tab: Tab) -> Self {
        TabGroup {
            tabs: vec![initial_tab],
            active: 0,
        }
    }

    /// Get the currently active tab.
    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    /// Get the currently active tab mutably.
    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Get the index of the active tab.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Get all tabs as a slice.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// Get all tabs as a mutable slice.
    pub fn tabs_mut(&mut self) -> &mut [Tab] {
        &mut self.tabs
    }

    /// Add a new tab and make it active.
    pub fn add_tab(&mut self, tab: Tab) {
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    /// Close the tab at the given index.
    /// Returns true if the tab was closed, false if it was the last tab.
    pub fn close_tab(&mut self, index: usize) -> bool {
        // Don't close the last tab
        if self.tabs.len() <= 1 {
            return false;
        }

        if index >= self.tabs.len() {
            return false;
        }

        self.tabs.remove(index);

        // Adjust active index if needed
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > index {
            self.active -= 1;
        }

        true
    }

    /// Close the currently active tab.
    /// Returns true if the tab was closed.
    pub fn close_active_tab(&mut self) -> bool {
        self.close_tab(self.active)
    }

    /// Switch to the next tab.
    pub fn select_next_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    /// Switch to the previous tab.
    pub fn select_previous_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active = if self.active == 0 {
                self.tabs.len() - 1
            } else {
                self.active - 1
            };
        }
    }

    /// Switch to a specific tab by index (0-based).
    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// Switch to the last tab.
    pub fn select_last_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = self.tabs.len() - 1;
        }
    }
}
