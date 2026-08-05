# Why Use This Fork?

This build of Alacritty focuses on an improved macOS experience. Here’s what you gain compared with the upstream project:

- **Modern app icon**: the icon is built with macOS Tahoe in mind.
- **Wide‑gamut rendering**: the rendering targets Display P3, so colors stay vibrant on Apple displays.
- **Sharper fullscreen**: the terminal now reaches every edge of the screen with no padding, maximizing lines and columns.
- Every macOS build is **signed** and **notarized by Apple**. You can install it without bypassing Gatekeeper prompts or running `xattr` commands.
- **Tabs** supported out of the box on all platforms. On macOS, set `tabs.enabled = true` to use the custom tab bar instead of native tabs (requires restart).
- Better font rendering on Windows
- **DPI-aware font switching**: configure different fonts for different scale factors using `[[font.dpi_override]]`. For example, use a retina-optimized font at 2x and a standard font at 1x.
- **Deferrable keybindings**: bindings marked `defer = true` are forwarded to applications that opt in (private mode 2064) and only run their terminal action when the application reports the key as unhandled — so `Cmd+C`/`Cmd+V` can reach TUI applications while keeping normal copy/paste everywhere else. See `docs/key-event-handling-reports.md`.
