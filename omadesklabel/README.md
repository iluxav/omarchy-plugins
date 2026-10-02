# Omadesklabel

The Omarchy workspace numbers, with a name of your own on each one.

```
1)omanote  2)my project  3  4  5
```

- **Left click** a workspace to go to it.
- **Right click** a workspace to label it. Type a name and press **Enter**:
  `1` becomes `1)omanote`. **Esc** cancels. Saving an empty name puts the plain
  number back.

Unlabelled workspaces look exactly like the stock `omarchy.workspaces` widget.
A labelled workspace keeps its text when focused and takes the bar's active
colour instead of turning into a dot. On a vertical bar the numbers stay, since
there is no room for words.

## Install

Link (or clone) this folder into the shell's plugin directory, then put it in
the bar in place of `omarchy.workspaces`:

```bash
ln -s ~/Projects/omadesklabel ~/.config/omarchy/plugins/iluxav.omadesklabel
```

```jsonc
// ~/.config/omarchy/shell.json
"left": [
  { "id": "omarchy.menu" },
  { "id": "iluxav.omadesklabel" }
]
```

The shell reloads plugins and `shell.json` on save.

## Settings

| Setting     | Default | Meaning                                       |
|-------------|---------|-----------------------------------------------|
| `maxLength` | `24`    | Longer labels are cut short with an ellipsis (hover shows the whole label). |

```json
{ "id": "iluxav.omadesklabel", "maxLength": 16 }
```

## Where the labels live

`~/.config/omadesklabel/labels.json`, keyed by workspace number:

```json
{
  "1": "omanote",
  "2": "my project"
}
```

You can edit it by hand; the bar follows the file as it changes.
