# Stadia Controller

Support for using the Stadia Controller as an Xbox 360 controller
on Windows.

### Supported features
- All buttons are mapped to their Xbox 360 equivalents.
  - Triggers are analog.
  - For the Assistant and Capture buttons which have no Xbox 360 equivalent,
    the command line flags `-capture-pressed`, `-assistant-pressed`, `-capture-released` and
    `-assistant-released` can be used to specify custom commands to run when those
    buttons are pressed and released.
    - For instance, `-capture-pressed "sharex -PrintScreen"` takes a screenshot when the Capture
      button is pressed.
- Vibrations are supported.
- Emulation via [ViGEm](https://github.com/ViGEm/Home) (must be installed), which means that
  everything just works. There won't be pesky Denuvo games that refuse to accept that input.

### Installation
1. Install [ViGEm](https://github.com/ViGEm/ViGEmBus/releases).
2. Download a release from the [releases](https://github.com/71/stadiacontroller/releases) page.
3. Extract the zip into a directory.

### Alternative
[XOutput](https://github.com/csutorasa/XOutput) does not support vibrations,
analog triggers and additional buttons, but it has more features and is more stable overall.
