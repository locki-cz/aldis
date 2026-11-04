# aldis

A tool for updating MCU firmware. `aldis` is named after the Aldis lamp, the signal lamp ships use to flash messages to each other.

<img src="images/demo.gif" alt="aldis update --pull --all --force" width="800">

## What It Does

`aldis` needs no per-MCU configuration: it discovers every supported MCU already configured in a running Klipper/Moonraker installation, compares each one's running firmware against the local Klipper source checkout, and builds and flashes the ones that are out of date. It stops Klipper only when a build or flash is about to happen, and it reports its plan before changing anything.

## Requirements

- Linux (uses udev and sysfs directly; the build fails on other platforms)
- A running Moonraker instance
- Each target MCU must already be running a version new enough to report `mcu_kconfig`:
  - Klipper: `v0.13.0-753-g8c29c0a8e` or newer
  - Kalico: `v2026.09.00-3-g6127720c4` or newer
  - MCUs running older firmware are reported as unsupported legacy and `aldis` will never offer to update them; flash them one last time by hand (e.g. Klipper's own `make flash`) to bring them under `aldis`.

## Installation

Download the latest build for your host's architecture (`aarch64`, `armv7l` for 32-bit Raspberry Pi OS, or `x86_64`) and extract it to `~/aldis`:

```
mkdir -p ~/aldis
curl -fL "https://github.com/mjonuschat/aldis/releases/latest/download/aldis-$(uname -m)-linux.tar.xz" \
  | tar xJ -C ~/aldis --strip-components=1
chmod +x ~/aldis/aldis
```

The rest of this document assumes `aldis` was installed this way, i.e. is not on `PATH` and is invoked as `~/aldis/aldis`.

## Setup

MCU updates need host permissions that a normal user does not have by default: udev rules so USB bootloader devices are accessible, and a narrow sudoers policy so Klipper can be stopped and started, and the Linux host MCU installed, without a password prompt mid-update.

```
sudo ~/aldis/aldis setup
```

installs both. Run `~/aldis/aldis setup --check` to verify they're in place without changing anything.

## Agent (web UI integration)

```
sudo ~/aldis/aldis setup --agent
```

Installs and starts the `aldis` service under the sudo user, so it survives reboots and keeps running. Adds it to Moonraker's service list (`moonraker.asvc`) so web UIs can manage and restart it from their service menus. Restart Moonraker once afterwards so it picks up the new entry.

```
sudo ~/aldis/aldis setup --agent --remove
```

Stops and removes the service and drops it from Moonraker's service list.

### Several printers

One agent can serve every Klipper/Moonraker pair on the host. List the Moonrakers in a TOML file:

```toml
moonraker = ["http://127.0.0.1:7125", "http://127.0.0.1:7126"]
```

Pass the file with `--config PATH`, or repeat `--moonraker URL`. Flags add to the file. To install the service for two printers:

```
sudo ~/aldis/aldis setup --agent --moonraker http://127.0.0.1:7125 --moonraker http://127.0.0.1:7126
```

Each printer has its own MCU status, firmware updates and history. Updates run one at a time across the host.

The same file tells the agent where Beacon and Cartographer firmware lives when it is not in the default places (see [Beacon](#beacon) and [Cartographer](#cartographer)):

```toml
beacon_source = "/home/pi/beacon_klipper"
cartographer_source = "/home/pi/cartographer_firmware"
```

The command-line tools take `--beacon-source` and `--cartographer-source` (or `ALDIS_BEACON_SOURCE` and `ALDIS_CARTOGRAPHER_SOURCE`) instead.

Klipper services other than `klipper` (such as `klipper-2` or `klipper@voron`) need sudo 1.9.10 or newer: Debian 12, Raspberry Pi OS Bookworm, Ubuntu 24.04. The default `klipper` service works with any sudo. With several printers configured, `setup --check` prints the sudo version.

Flashing a USB-CAN bridge resets its CAN bus, so aldis refuses while another printer on that bus is printing, starting up, or cannot be checked. It stops nothing on the other printers, and their Klipper may need a `FIRMWARE_RESTART` once the bridge is back. `update` and `flash` run the same check when given `--config PATH` (or `ALDIS_CONFIG`). Without it they look only at the printer named by `--moonraker`.

## Usage

```
~/aldis/aldis status                    # discovered MCUs and whether firmware is current
~/aldis/aldis inspect                   # MCU configuration reported by Moonraker
~/aldis/aldis update                    # build and flash outdated, supported MCUs (prompted per MCU)
~/aldis/aldis update <mcu> [<mcu> ...]  # build and flash specific MCUs
~/aldis/aldis update --all              # build and flash every supported MCU, even ones already current
~/aldis/aldis update --auto             # download Klipper/Kalico updates, then build and flash everything outdated, no prompts
~/aldis/aldis update --force            # update every eligible MCU, same as --all, but also works with specific <mcu> targets
~/aldis/aldis self-update               # check for and install a newer aldis release
~/aldis/aldis self-update --check       # report whether a newer release is available, without installing it
```

`update` requires the host permissions installed by `~/aldis/aldis setup` (see above). A few flags interact:

- `--force` alone updates every eligible MCU, same as `--all`. Combined with specific `<mcu>` names, it force-updates just those, even if already current.
- `--pull` can be combined with specific `<mcu>` names too.
- `--auto` already implies `--pull` and can't be combined with `--pull`, `--force`, `--all`, or explicit MCU names.

Each build writes the MCU's embedded Kconfig to `.config` in the Klipper checkout, replacing whatever `make menuconfig` left there; keep a copy of a hand-maintained `.config` if you need it. The run's workspace keeps the configuration as expanded by `make olddefconfig`.

`self-update` compares the running binary's version against the latest GitHub release and, unless already current, downloads and installs the matching platform archive over the current binary. The archive's SHA-256 digest must match the `SHA256SUMS` file published with the release, or nothing is installed. No `sudo` is required as long as `~/aldis` is writable by the current user. If the agent service is running, `self-update` restarts it onto the new binary; otherwise it leaves the service alone.

`status` also reports MCUs Klipper configured but could not reach, with their connection info and why. `update` and `flash` refuse to run against a remote Moonraker or one whose Klipper service name is not a valid Klipper unit. Only one `update`, `flash`, or `reboot` runs at a time on the host, and a second one fails immediately instead of racing the first.

## Supported Flash Backends

These bootloaders are entered and flashed unattended, directly from the running Klipper application:

- **Katapult**, over USB serial, a plain UART (such as a Pi's GPIO UART), or CAN
- **STM32 ROM DFU** (`0483:df11`)
- **PicoBoot** (RP2040/RP2350, `2e8a:0003`/`2e8a:000f`)
- **SAM-BA** on SAMD21 (the Seeed XIAO SAMD21's UF2 bootloader, `2886:002f`)

FOCI firmware is flashed through Katapult only; see [FOCI](#foci). Beacon firmware is flashed
through Beacon's own DFU bootloader only; see [Beacon](#beacon). Cartographer firmware is
flashed through Katapult only, and only when the probe is named; see
[Cartographer](#cartographer).

Every other bootloader identity is reported as unsupported rather than guessed at. MCUs whose
firmware configuration names a family none of these bootloaders run on (Katapult builds only for
STM32, RP2040/RP235x, and LPC176x) are reported as unsupported up front and never offered an
update; SAMD51 and other non-SAMD21 ATSAMD boards are the common case.

A SAMD21 board must have been built with a bootloader offset and, for UF2 bootloaders such as the
XIAO's, with the matching `Bootloader type` (Kalico) so that the 1200-baud request lands in the
bootloader rather than rebooting into the application.

Only the XIAO's bootloader USB ID is recognised. Other SAMD21 boards are reported as an
unsupported bootloader until their ID is added and the flash is tested on that board.

### Katapult over a UART

When an MCU's `serial:` is a plain tty with no USB device behind it (for example `/dev/ttyAMA5`),
`aldis` sends Klipper's serial bootloader request, waits for Katapult to answer on the same tty, and
flashes only after it has. Requirements:

- Katapult must be built for the same UART and baud rate as the running firmware. `aldis` reads the
  baud rate from the firmware's embedded configuration (`CONFIG_SERIAL_BAUD`, 250000 when absent) and
  cannot read Katapult's own setting. If they differ, `aldis` stops with "the MCU did not enter
  Katapult" before sending any flash command, leaving the MCU waiting in Katapult.
- The host user needs access to the tty (typically the `dialout` group), and the UART must be enabled
  on the host and not used as a console.
- `aldis reboot` finds stuck bootloaders by scanning USB, so a Katapult left waiting on a UART is not
  listed. Run `flash` against the MCU again to recover it.

## FOCI

[FOCI](https://github.com/foci-rs/foci) MCUs are not built from the Klipper checkout. `aldis` updates a FOCI release build to the latest FOCI release instead:

- `status` and `update` look up the latest release on GitHub when Moonraker reports an MCU whose firmware names itself `FOCI`, and show it as `FOCI release`. A failed lookup is reported, and the FOCI MCUs' update state stays unknown. Nothing is looked up without a FOCI MCU.
- The board comes from the firmware's `MCU` (`stm32h723xx` is Ouroboros, `stm32f407xx` is OpenFFBoard) and the flavor from its build features (`prod`, or `trace`). The update downloads that board's and flavor's `.bin` from the release, checks it against the `.sha256` published beside it, and keeps a copy in the run workspace.
- Only release builds are compared with the release: a development or dirty build, or one newer than the release, shows as unknown and is updated only with `--all` or `--force`. A build with other features (for example `dev`) has no release to update to and is reported as unsupported.
- The image is sent only to Katapult, over the board's USB serial device, and only when Katapult starts applications where the image is linked to run (`0x08020000` on Ouroboros, `0x08008000` on OpenFFBoard). Otherwise nothing is written and Katapult is asked to start the existing application.
- Keep the `klipper-foci` host module in step with the firmware: update it (for example through Moonraker's update manager) before updating the firmware, including before `update --auto`.

`flash` sends a firmware file for a FOCI MCU through the same Katapult-only path. The agent compares FOCI MCUs with the latest release and offers to update outdated ones; it asks GitHub at most once an hour (every five minutes while lookups fail).

## Beacon

[Beacon](https://github.com/beacon3d/beacon_klipper) probes are not built from the Klipper checkout either. Their firmware ships as one image per hardware revision (`firmware/rev<x>.dfu`) in the `beacon_klipper` checkout, which is updated with `git pull`. `aldis` flashes a Beacon with the image for its revision from that checkout:

- The checkout is the one Beacon's installer linked `klippy/extras/beacon.py` from in the Klipper checkout, otherwise `~/beacon_klipper`. Name another with `--beacon-source` (or `ALDIS_BEACON_SOURCE`), or `beacon_source` in the agent's configuration file. `status` shows it as `Beacon firmware`.
- A Beacon is recognized by the version it reports (`Beacon 2.1.0`). Its revision comes from the USB product name of the device behind its configured `serial:` (`Beacon RevH` takes `revh.dfu`), and the image's version from its DFU suffix, whose checksum must match.
- As with Beacon's own `update_firmware.py`, any difference between the running version and the image counts as an update, so checking out an older `beacon_klipper` commit downgrades.
- The update reads the image right before flashing it and refuses it if the checkout now holds another version than the one approved. It keeps a copy in the run workspace.
- The Beacon is asked into its bootloader with a 1200-baud touch, and the image is downloaded with plain USB DFU only to a bootloader its DFU suffix names (`04d8:beac` or `04d8:bea1`). A Beacon on another transport is refused before it is touched. The image is encrypted, so it cannot be read back; success is confirmed by the Beacon reporting the image's version after Klipper restarts.
- The images are Beacon's and not freely redistributable, so `aldis` only ever reads them from your own checkout. It never downloads or copies them anywhere else.
- `aldis setup` grants access to the Beacon bootloader; rerun it after upgrading `aldis`.

`flash` accepts only a Beacon `.dfu` image for a Beacon, including one whose version `aldis` cannot compare (such as a beta), and sends it through the same route. The agent compares Beacons with their images and offers to update outdated ones.

A Beacon left in its bootloader (for example by an interrupted flash) can only leave it by being flashed. `aldis reboot` finds it on USB and, after asking, flashes the image for the revision its bootloader names, from the same checkout (`--beacon-source` names another). It flashes nothing when the checkout's image is not for that bootloader. Restart Klipper afterwards (`FIRMWARE_RESTART`) so it reconnects the Beacon.

## Cartographer

[Cartographer](https://github.com/Cartographer3D/cartographer_firmware) probes are not built from the Klipper checkout either. Their firmware is published as prebuilt images in the `cartographer_firmware` repository, one per probe generation, link (USB, or CAN at one bitrate) and build (full or lite), listed in its `firmware_list.csv`. `aldis` flashes a Cartographer with an image from your own clone of it:

- The clone is `~/cartographer_firmware`; name another with `--cartographer-source` (or `ALDIS_CARTOGRAPHER_SOURCE`), or `cartographer_source` in the agent's configuration file. `status` shows it as `Cartographer firmware`, with the installed `cartographer3d-plugin` version, read from the virtualenv Klippy runs under (or `~/klippy-env`).
- A Cartographer is recognized by its `app` (`Cartographer`, from 6.1.0) or, before that, by a version starting with `Cartographer`. Its generation comes from its `MCU` (`stm32f042x6` is V2/V3, `stm32g431xx` V4), its build from its version (`lite`, or `K1` for the 5.x V3 builds, is the lite build), and its link and bitrate from its `CANBUS_FREQUENCY`. A version without a number (`CARTOGRAPHER V3`) cannot be compared and is reported as unsupported.
- Only images the manifest lists as updates onto an existing Katapult are considered, never one that includes or replaces the bootloader. A wrong image leaves a Cartographer recoverable only with its boot pins bridged by hand, so the manifest only nominates candidates: each image's own embedded identify dictionary must name exactly the probe's `MCU`, CAN bitrate and build, and the version the manifest gives, and the image must be linked to start at `0x08002000`, after Cartographer's 8 KiB Katapult. Exactly one image may match.
- The image is the newest the installed plugin works with (`min_plugin_version`); `status` names a newer one the plugin is too old for. Update the plugin yourself first; `aldis` never installs it. A probe newer than its image is not downgraded unless `--force` is given.
- Cartographer's plugin removes every scan and touch model made with other firmware, so every update means recalibrating. A Cartographer is therefore updated only when named (`aldis update cartographer`), never by `update` on its own, `--all` or `--auto`, which say how to update one that is behind. The confirmation prompt and the end of the run say it must be recalibrated.
- The image is read right before flashing, checked once more against the probe, and refused if the clone now holds another image than the one approved. It is sent only to Katapult, over USB or CAN, and only to a Katapult that starts applications at `0x08002000`; a Cartographer on a UART is refused before it is touched. A copy is kept in the run workspace.
- The images are Cartographer's; their repository carries no license file, so `aldis` only ever reads them from your own clone and never downloads them.

`flash` accepts for a Cartographer only an image whose identify dictionary names its own `MCU`, link and CAN bitrate and that starts at `0x08002000`; a file for the other build (full or lite) is allowed. The agent reports Cartographers as current or behind their image but offers no update action; update them with `aldis update` and their name.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
