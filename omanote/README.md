# omanote (omaestro plugin)

SUPER+N opens the Omanote popup from the Omarchy bar: type to find a note,
`Enter` to open it or start a new one, `>` to search inside the notes.

This is an [omaestro](https://github.com/iluxav/omaestro) plugin, so the
hotkey is registered live and never written to the Hyprland config. It needs
the `iluxav.omanote` shell plugin to be enabled.

## Install

```sh
om plugin add ~/Projects/omarchy-plugins/omanote
```

## Options

Written to `~/.config/omaestro/rules.d/omanote.lua`; `om plugin configure
omanote` opens them as a form.

- `chord`: open or close the notes popup (default `SUPER + N`)
- `capture`: a second chord for the one-line quick-capture box (default none;
  `SUPER + SHIFT + N` is Omarchy's Editor shortcut, so pick a free one)
