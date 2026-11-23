# Accessibility

State after the RC accessibility pass (2026-11-08). Labels follow
the project's evidence rules: *Verified* = exercised by a test or
by construction in code that a test covers; *Unverified* = true by
code inspection, still to be confirmed on a real desktop.

## What holds

- **Native controls throughout** (Verified by construction). Every
  window uses standard Windows controls, which bring screen-reader
  support, focus rectangles and high-contrast behaviour with them.
- **Keyboard navigation in task windows** (fixed in RC). The
  message loop ran dialog navigation (Tab, arrow keys, mnemonics)
  only for the main window, so task windows could not be driven by
  keyboard alone. It now runs against whichever window a message
  belongs to.
- **Menu mnemonics** (Verified by inspection). Menus and radio
  groups carry `&` access keys; the main grid is fully keyboard
  driven (type to filter, arrows to move, Enter to open, context
  key for the variant menu).
- **Names for the task grid** (Verified). Grid buttons expose an
  accessible name that includes the task title and its state.
- **Status is text, never colour alone** (Verified). Job progress
  and results are written as plain sentences in each window's
  status line and in the main log; the log can be filtered by
  severity and copied.
- **FluentGruv contrast** (Verified in palette tests). Dark and light
  tokens match the published FluentGruv ratios; high-contrast mode uses
  Windows system colors. Runtime captures still need a Windows 11 x64
  check after this UI pass. Text size follows Windows by default and can
  be scaled 100–200% in Options.
- **Workspace navigation and drawers** (Implemented; native runtime check pending).
  Tasks and Activity have labeled keyboard-accessible tabs; Details and
  Logs drawers close with Esc, and drawer state is saved locally.
- **Scroll discipline** (Verified by layout tests). The main task grid
  never scrolls. Only activity and log text areas have their own
  vertical scrollbar.

## Still owed (field checks)

- A full pass with a screen reader (Narrator or NVDA) over the
  main window and each task window.
- Keyboard-only completion of one full job in a task window.
- 200–300% display scaling and Windows high-contrast themes.
- Visual reflow at 1024×700, 1280×800, 1440×900 and 1920×1080, including
  an open drawer on each screen size.

These need a real Windows desktop; results will be recorded here
when they happen.
