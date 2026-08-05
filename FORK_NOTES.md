# Why Use This Fork?

This build of Alacritty focuses on an improved macOS experience. Here’s what you gain compared with the upstream project:

- **Modern app icon**: the icon is built with macOS Tahoe in mind.
- **Wide‑gamut rendering**: the rendering targets Display P3, so colors stay vibrant on Apple displays.
- **Sharper fullscreen**: the terminal now reaches every edge of the screen with no padding, maximizing lines and columns.
- Every macOS build is **signed** and **notarized by Apple**. You can install it without bypassing Gatekeeper prompts or running `xattr` commands.
- **Tabs** supported out of the box on all platforms. On macOS, set `tabs.enabled = true` to use the custom tab bar instead of native tabs (requires restart).
- Better font rendering on Windows
- **DPI-aware font switching**: configure different fonts for different scale factors using `[[font.dpi_override]]`. For example, use a retina-optimized font at 2x and a standard font at 1x.
- **Deferrable keybindings**: applications that enable Kitty keyboard flag 128 receive all physical keys through a universal CSI-u format; bindings marked `defer = true` carry a nonzero report id and run their terminal action only when the application reports the key as unhandled. This lets `Cmd+C`/`Cmd+V` reach TUI applications while keeping normal copy/paste everywhere else. See `docs/key-event-handling-reports.md`.
