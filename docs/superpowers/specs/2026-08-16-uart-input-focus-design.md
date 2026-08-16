# UART Input Focus Design

## Goal

Make the Playground UART terminal usable without clicking the input field after
every command. Typing in the application window should naturally continue in
the UART input, and submitting a command should provide the line terminator
expected by terminal firmware such as NuttX NSH.

## Chosen behavior

- The UART input requests keyboard focus when a VM session is available.
- It regains focus after submission and after interactions with non-text UI.
- Clicking toolbar buttons remains possible; focus is restored on the following
  frame rather than interfering with the click.
- Enter and the Send button have identical behavior.
- A non-empty submission sends the UTF-8 input bytes followed by `\n`, then
  clears the visible input.
- Empty submissions do nothing and do not inject an unsolicited newline.
- Normal text-edit shortcuts, including clipboard shortcuts, continue to be
  handled by the input widget.

## Implementation

Keep line construction in `UartInputState`, independent of egui, so newline and
clearing semantics have direct unit coverage. Give the input widget a stable
egui ID. When the VM is present and no other text-edit widget needs focus,
request focus for that ID each frame; after either submission path, request it
again explicitly.

The terminal output remains read-only and existing runtime/UART interfaces do
not change.

## Verification

- A unit test proves non-empty submissions append exactly one newline and clear
  the input.
- A unit test proves empty submissions send nothing.
- Existing UI, runtime, UART firmware, profile, and NuttX tests remain green.
- Manual behavior: launch the NuttX profile, type `help`, press Enter, then type
  another command immediately without clicking the field.
