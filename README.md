# Elden Ring Nightreign — Freecam

A standalone, high-performance Freecam and camera manipulation toolkit for **ELDEN RING: NIGHTREIGN**, written in Rust.

Designed for virtual photography, cinematic content creation, and world inspection with pure keyboard & mouse control, zero-velocity Havok teleportation, fall-death prevention, and an integrated translucent in-game OSD guide.

---

## Supported Game Versions

> [!IMPORTANT]
> This mod is strictly and exclusively compatible with:
> - **ELDEN RING: NIGHTREIGN version `1.3.3.0`**
> - **ELDEN RING: NIGHTREIGN version `1.3.2.0`**
>
> Other game versions are not supported due to game memory offsets and engine structures.

---

## Features

- **Pure Keyboard & Mouse Control:** Complete 6-axis camera navigation (movement, elevation, rotation, roll) with zero joystick dependency.
- **360° Unconstrained Free Look:** Unlimited pitch, yaw, and roll without mouse deadzones or viewport locking.
- **Translucent In-Game Guide (OSD):** Built-in layered HUD guide displaying keybindings on screen. Uses `WS_EX_TRANSPARENT` pass-through, meaning clicks pass directly into the game without stealing focus. Automatically closes when the game exits.
- **Safe Teleport (`T`):** Teleports the character precisely to the camera's coordinates with zero residual root-motion velocity and immediately disables freecam for a seamless transition.
- **Permanent Fall Death Protection:** Fall-death timers are suppressed so the player character never dies from long drops while navigating or landing.
- **Emergency Rescue (`Home` / `Backspace`):** Instantly teleports the character back to the initial spawn position in case of accidental drops out of bounds.

---

## Controls

All controls are operated via Keyboard and Mouse:

| Key / Input | Action | Description |
| :--- | :--- | :--- |
| **`P`** or **`F1`** | **Toggle Freecam** | Activates or deactivates the free camera mode |
| **`W`**, **`A`**, **`S`**, **`D`** | **Move Camera** | Fly forward, strafe left, backward, or right |
| **`Space`** | **Fly Up** | Elevate camera vertically |
| **`Ctrl`** or **`C`** | **Fly Down** | Lower camera vertically |
| **`Shift`** | **Speed Boost** | Accelerate camera speed by 3.5× |
| **`Alt`** | **Precision Speed** | Slow camera speed by 0.25× for fine positioning |
| **`Mouse`** | **Look 360°** | Full camera orientation (Yaw and Pitch) |
| **`Q`** / **`E`** | **Roll Camera** | Tilt camera angle left or right |
| **`R`** | **Reset Roll** | Reset camera roll back to 0° level horizon |
| **`T`** | **Teleport & Land** | Teleport character to camera position, land safely, and exit freecam |
| **`Home`** or **`Backspace`** | **Rescue to Origin** | Emergency return to the initial world spawn location |
| **`H`** or **`F2`** | **Toggle Guide (OSD)** | Show or hide the translucent on-screen key guide |

---

## Installation & Setup

No installation scripts or batch files are required. You only need two files:

1. **Download:** Grab the latest release from the [Releases](https://github.com/carlosjrgit/Nightreign_Freecam/releases) tab:
   - `FreecamLauncher.exe`
   - `Freecam.dll`
2. **Copy:** Place both `FreecamLauncher.exe` and `Freecam.dll` directly inside your game directory (the folder containing `nightreign.exe`).
3. **Run the Game:** Start *ELDEN RING: NIGHTREIGN* and load your character into the game world.
4. **Launch Freecam:** Double-click `FreecamLauncher.exe` (run as Administrator if your game runs with elevated privileges).
5. **Enjoy:** Return to the game, press **`P`** or **`F1`** to toggle Freecam, and press **`H`** or **`F2`** to show/hide the on-screen guide!

---

## Building from Source

To compile the binaries yourself from the source code:

### Prerequisites
- [Rust](https://www.rust-lang.org/) (2021 edition or newer)
- Target toolchain for Windows: `x86_64-pc-windows-gnu` or `x86_64-pc-windows-msvc`

### Build Command
```powershell
# Clone the repository
git clone https://github.com/carlosjrgit/Nightreign_Freecam.git
cd Nightreign_Freecam

# Build the release profile
cargo build --target x86_64-pc-windows-gnu -p nightreign-freecam --release
```

The compiled binaries will be generated at:
- Launcher: `target/x86_64-pc-windows-gnu/release/FreecamLauncher.exe`
- Library: `target/x86_64-pc-windows-gnu/release/nightreign_freecam.dll` (rename to `Freecam.dll` for distribution)

---

## Project Structure

```text
├── crates/
│   ├── eldenring/           # Elden Ring engine structure definitions
│   ├── nightreign/          # Nightreign memory signatures, RVA tables & classes
│   └── shared/              # Shared FromSoftware runtime utilities & math
├── examples/
│   └── nightreign-freecam/  # Main Freecam project
│       ├── src/
│       │   ├── bin/
│       │   │   └── injector.rs   # FreecamLauncher.exe source
│       │   └── lib.rs            # Freecam.dll source (camera logic, Havok sync & OSD)
│       └── Cargo.toml
├── Cargo.toml
└── README.md
```

---

## Security & Integrity

- Every binary asset published in official [GitHub Releases](https://github.com/carlosjrgit/Nightreign_Freecam/releases) is accompanied by verified **SHA-256 checksums**.
- Source code is completely open for public inspection and contains zero proprietary keys, telemetry, or external network requests.
- Binaries are unsigned open-source tools; standard Windows SmartScreen alerts can be bypassed by clicking *"More info"* → *"Run anyway"*.

---

## License & Credits

- Underlying engine reverse-engineering runtime bindings built on `fromsoftware-rs` by `vswarte`.
- Community reverse engineering contributions by Dasaav, Tremwil, Sfix, Yui, and Vawser.
- Licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-ASL2).
