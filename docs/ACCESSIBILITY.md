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
- **Readable contrast** (Verified on captures). The charcoal/red
  theme was corrected once already from real captures (0.4.0) so
  secondary text stays legible; text size follows the Windows
  setting by default and can be scaled 100–200% in Options.

## Still owed (field checks)

- A full pass with a screen reader (Narrator or NVDA) over the
  main window and each task window.
- Keyboard-only completion of one full job in a task window.
- 200–300% display scaling and Windows high-contrast themes.

These need a real Windows desktop; results will be recorded here
when they happen.
