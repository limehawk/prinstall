# Lazy-Style TUI Design Guide

Reference guide for building prinstall's TUI in the style of lazygit/lazydocker.

## Core Layout Pattern

```
┌─────────────────────────────────────────────────────────┐
│ Header / Breadcrumb Bar                                 │
├────────────┬────────────────────────┬───────────────────┤
│            │                        │                   │
│  Sidebar   │     Main Panel         │  Detail / Preview │
│  (list/    │     (content,          │  (contextual      │
│   tree)    │      table, log)       │   info, diff,     │
│            │                        │   preview)        │
│            │                        │                   │
├────────────┴────────────────────────┴───────────────────┤
│ Status Bar / Command Palette / Keybinding Hints         │
└─────────────────────────────────────────────────────────┘
```

## Ratatui Widget Usage (framework-first)

- `List` / `ListState` — sidebar navigation, selection tracking
- `Table` / `TableState` — printer list, driver list
- `Paragraph` with `Scroll` — detail panels, log views
- `Tabs` — view switching
- `Block` — every panel wrapped with borders, titles
- `Layout` + `Constraint` — panel splitting (never manual geometry)
- `Gauge` / `LineGauge` — install progress
- `Scrollbar` / `ScrollbarState` — paired with scrollable content
- Third-party: `tui-input` for subnet input, `throbber-widgets-tui` for spinners

## Keybindings (vim-style)

| Key | Action |
|-----|--------|
| j/k | Move down/up in lists |
| h/l | Move focus left/right between panels |
| g/G | Jump to top/bottom |
| Enter | Select/confirm/drill into |
| / | Open search/filter |
| Esc | Back/cancel/close overlay |
| q | Quit |
| ? | Help overlay |
| Tab/Shift+Tab | Cycle panel focus |
| Space | Toggle selection |
| [/] | Switch tabs/views |

## Visual Design

- Terminal default background (no explicit bg color)
- Focused panel: bright/accent border; unfocused: dim border
- Accent color for focused borders, selected items, active tabs
- Semantic: red=error/destructive, yellow=warning, green=success
- Dense layout — every row carries information
- Truncate with … in lists/tables, wrap only in detail panels
- 1-space padding inside panels
- Right-align numbers/timestamps, left-align names

## Interaction Patterns

- One panel has focus at a time
- Sidebar drives navigation — selecting updates main + detail
- Status bar always present: context, key hints, transient messages
- Loading: spinner for >200ms, progress message for >2s
- Errors: transient flash in status bar (red, auto-dismiss 3-5s)
- Confirmation dialogs for destructive actions: `Are you sure? [y/N]`
- Filter: / activates, fuzzy by default, Esc clears, show match count
