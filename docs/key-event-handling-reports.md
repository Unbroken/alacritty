# Key event handling reports

An extension to the [kitty keyboard protocol] that lets an application take over specific
terminal keybindings — for example `Cmd+C`/`Cmd+V` on macOS — while keeping the terminal's
behavior for applications that do not care. The terminal defers marked keybindings, forwards
the key to the application, and runs its own action only when the application reports the key
event as unhandled (or does not answer in time).

[kitty keyboard protocol]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/

## Opt in

The application enables reports with a DEC private mode:

| Sequence         | Meaning                             |
| ---------------- | ----------------------------------- |
| `CSI ? 2064 h`   | Enable key event handling reports   |
| `CSI ? 2064 l`   | Disable key event handling reports  |
| `CSI ? 2064 $ p` | Query support and state (`DECRQM`)  |

The `DECRQM` reply is `CSI ? 2064 ; Ps $ y` with `Ps` = `1` (set), `2` (reset), or `0` from
terminals without the extension. The mode is reset by a full terminal reset (`RIS`).

Applications should also enable the kitty keyboard protocol (at least *disambiguate escape
codes*): keys such as `Cmd+C` only have an escape sequence encoding under that protocol, and
deferral silently degrades to normal terminal keybinding behavior for keys the terminal cannot
forward.

## Key events

While the mode is set, a key event matching a keybinding configured with `defer = true` is not
executed immediately. Instead the terminal forwards the key in kitty encoding with the report
id appended as a fourth parameter:

```
CSI unicode-key-code[:shifted[:base]] ; modifiers[:event-type] ; text-codepoints ; id u
```

Empty parameters keep their kitty defaults, so `Cmd+C` arrives as e.g. `CSI 99;9;;42u`
(no associated text). Ids are nonzero and assigned by the terminal; applications must treat
them as opaque. Only key events carrying an id expect an answer — everything else behaves
exactly as in the kitty protocol.

## Reports

After dispatching the key, the application answers:

```
CSI ? id ; handled u
```

with `handled` = `1` when it consumed the key, `0` when it did not. On `0` the terminal runs
the deferred keybinding action. On `1` it does nothing. If no report arrives within 250 ms the
terminal treats the key as unhandled and runs the action, so a hung or killed application does
not lose the user's copy/paste keys.

The parameterless form `CSI ? u` remains the kitty keyboard mode query; the parameters
distinguish the report.

## Alacritty configuration

```toml
[keyboard]
bindings = [
    { key = "C", mods = "Command", action = "Copy",  defer = true },
    { key = "X", mods = "Command", action = "Copy",  defer = true },
    { key = "V", mods = "Command", action = "Paste", defer = true },
]
```

`defer` is only valid on key bindings. When the application has not enabled mode 2064 (or the
key has no kitty encoding), a deferred binding behaves exactly like a normal one, so the
configuration is safe to use unconditionally.

With tabs, replies resolve against the tab that forwarded the key; a deferred action whose tab
is no longer active when the reply or timeout arrives is dropped rather than executed against
the wrong terminal.

## Design notes

Terminal keybindings cannot synchronously ask the application about every key — an application
on the other end of an SSH connection would add its round trip time to every keystroke. This
extension confines the wait to the chords the user explicitly marked, makes it opt-in per
application, and bounds it with a timeout, which keeps the terminal responsive by construction.
