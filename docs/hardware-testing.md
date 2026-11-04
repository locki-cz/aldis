# Hardware testing: Beacon, Cartographer and FOCI

Support for these three devices was built from their projects' code, published images and update
scripts, and has unit tests, but has not yet been run against real hardware. This checklist names
what to try on each device, what to expect, and what to report back. It goes from steps that change
nothing to ones that flash firmware.

## Before you start

- Run the `aldis` build under test, and rerun `sudo aldis setup` with it: it installs the udev rules
  for the Beacon bootloader. `aldis setup --check` should report the udev rules as ready.
- Run every command with `-vv` and keep the output. `update` prints its run log's path at the end
  (`Run log: ...`); keep that file too. `klippy.log` around each flash helps as well.
- Have each project's own recovery tool to hand before flashing anything:
  - Beacon: `~/beacon_klipper/update_firmware.py update all`.
  - Cartographer: `~/cartographer_firmware/fw_update.sh`. For a probe that no longer starts at
    all, you need Cartographer's DFU recovery with the boot pins bridged by hand.
  - FOCI: Katapult's `flashtool.py`, using the `.bin` from the release.
- Nothing else may be printing on the host while a test flashes.

When something does not match what is expected, report the step, the full command output, the run
log and, where a step asks for it, `lsusb -v` output.

## Beacon

Needs a `beacon_klipper` checkout: the one `~/klipper/klippy/extras/beacon.py` is linked from, or
`~/beacon_klipper`. Run each step once for every Beacon (`mcu beacon`, `mcu beacon sensor <name>`).

1. **Status.** Run `aldis -vv status`.
   - Expect `Beacon firmware: <checkout>` near the top.
   - Under `mcu beacon`, expect:
     - `firmware: Beacon`;
     - `connection: serial (...)`, the `serial:` from your `[beacon]` section;
     - `supported: yes (revh.dfu holds Beacon 2.1.0)`, or `revd.dfu`, depending on the revision.
   - **Report:** the exact `version:` line. aldis expects `Beacon X.Y.Z`; anything else makes it
     report the Beacon as third-party firmware.
   - **Report:** the application's USB identity, from
     `cat /sys/bus/usb/devices/*/product /sys/bus/usb/devices/*/idProduct` or `lsusb -d 04d8:`.
2. **Bootloader identity.**
   - Stop Klipper (`sudo systemctl stop klipper`).
   - Enter the bootloader by hand with `stty -F /dev/serial/by-id/<beacon> 1200`.
   - **Report:** `lsusb -v -d 04d8:` while it is in the bootloader. Of interest:
     - the product string: aldis expects `Beacon Rev<X> ... Bootloader`;
     - `idProduct`: aldis expects `beac` or `bea1`;
     - the DFU functional descriptor: its `bLength` (7 means DFU 1.0, 9 means DFU 1.1),
       `bcdDFUVersion`, `bmAttributes` (manifestation tolerant, will detach), and `wTransferSize`.
3. **Recovery from the bootloader.** Leave the Beacon in its bootloader from step 2 and start
   Klipper (`sudo systemctl start klipper`); it will fail to reach the Beacon.
   - Run `aldis -vv reboot`.
   - Expect `Beacon bootloader at 04d8:bea1 (...)`, a prompt to flash `.../revh.dfu (Beacon 2.1.0)`,
     `flashed N bytes`, then `back in its application`.
   - Then expect a prompt to restart Klipper. After the restart, `aldis status` should show the
     Beacon again.
   - **Report:** whether the download ended cleanly. The run output says so, and with `-vv`, the log
     says whether the bootloader "dropped off the bus during manifestation".
   - **Report:** how long the Beacon took to come back.
4. **Same-version update.** Run `aldis -vv update --force mcu beacon`.
   - Expect these steps: `stopping Klipper`, `reading firmware image`, `entering bootloader`,
     `bootloader ready`, `flashing firmware`, `flashed N bytes`, `MCU restart confirmed`,
     `Klipper ready`, `all updated MCUs connected`.
5. **Real update.** In the checkout, go back one firmware release, from 2.1.0 to 2.0.1:
   `git -C ~/beacon_klipper checkout b30f733~1 -- firmware`.
   - `aldis status` should now show the Beacon as needing an update to `Beacon 2.0.1`. That is a
     downgrade, as upstream would also do.
   - Run `aldis update mcu beacon` and confirm.
   - Then restore the checkout with `git -C ~/beacon_klipper checkout HEAD -- firmware` and update
     again.
6. **File flash.**
   - `aldis flash mcu beacon ~/beacon_klipper/firmware/revh.dfu` should flash like step 4.
   - Any non-`.dfu` file should be refused before the Beacon is touched (`refusing to flash ...`).
7. **Agent.** With the checkout one release behind (step 5), the agent's `status` should show the
   Beacon as `update_available` with the `update` action, and an update from the frontend should
   behave like step 4.

## Cartographer

Needs a clone of `cartographer_firmware` in `~/cartographer_firmware`, pulled recently. Note the
plugin version with `~/klippy-env/bin/pip show cartographer3d-plugin`. Test a USB probe and a CAN
probe separately if you can, and on CAN note the bitrate.

Updating to another firmware version makes the plugin remove your scan and touch models. Steps 1
to 4 do not change the version. Step 5 does, so you will have to recalibrate after it.

1. **Status.** Run `aldis -vv status`.
   - Expect `Cartographer firmware: <clone>` and `Cartographer plugin: <version>`.
   - Under `mcu cartographer`, expect `supported: yes, when named (<image>.bin holds <version>)`.
   - **Report:** the exact `version:`, `model:` and `connection:` lines.
   - **Report:** whether the named image is the one `fw_update.sh` recommends for your probe: same
     generation, USB or CAN, bitrate, and full or lite.
2. **Never updated unless named.** `aldis update`, `aldis update --all` and `aldis update --auto`
   should not offer the Cartographer. If it is behind its image, they say instead that it is
   updated only when named.
3. **Refusals.**
   - `aldis flash mcu cartographer <image for another link or bitrate>` should be refused before
     the probe is touched: `does not suit it: it is built for ...`.
   - So should a combined image that includes Katapult, such as one from
     `firmware/v4/combined-firmware/`: `not linked to start at 0x8002000`.
4. **Same-version reflash.** Use the image of the version the probe already runs; pick it with
   `aldis flash mcu cartographer <that .bin>`.
   - Expect the confirmation prompt to warn about recalibration, then `entering bootloader`,
     `bootloader ready`, `flashing firmware`, `flashed N bytes`, `Klipper ready`.
   - On CAN, the probe is reached through Katapult on CAN, and Katapult must report its application
     at `0x8002000`.
   - **Report:** whether the plugin kept your models. It should, because the version string did not
     change.
   - **Report (CAN):** whether a USB-CAN bridge on the same bus stayed up throughout.
5. **Real update, only if you are ready to recalibrate.** Run `aldis update mcu cartographer`.
   - Expect the prompt and the end of the run to say the probe must be recalibrated.
   - After the restart, the probe must report exactly the image's version string.
   - **Report:** whether the plugin removed the models, and, with the legacy `mcu scanner` plugin,
     whether Klipper started at all.
6. **Plugin minimum.** With a plugin older than 1.6.0 and a V4 probe, `status` should pick the
   newest 6.1.0 image and say that 6.2.0 needs `cartographer3d-plugin 1.6.0 or newer`.
7. **Agent.** The agent's `status` should show a Cartographer that is behind its image as
   `update_available` with no actions, and a message saying to run `aldis update cartographer`.
   An update request naming it should be rejected as `not_updatable`.

## FOCI

Needs network access to GitHub. These steps work on Ouroboros (STM32H723) and OpenFFBoard
(STM32F407), over USB.

1. **Status.** `aldis -vv status` should show `FOCI release: v<latest>`, and for the board
   `supported: yes (FOCI ouroboros-fw prod release builds)`. **Report:** the exact `version:` line
   and the board's `mcu_build_versions`, from
   `curl -s 'http://127.0.0.1:7125/printer/objects/query?mcu%20<name>'`.
2. **Update.** First update the `klipper-foci` host module to match the release. Then run
   `aldis -vv update mcu <name>`.
   - Expect `downloading firmware`, `downloaded firmware`, `entering bootloader`, `bootloader ready`,
     `flashing firmware`, then the reconnect at the release's version.
   - Katapult must report its application at `0x8020000` (Ouroboros) or `0x8008000` (OpenFFBoard).
     Any other offset is refused before anything is written.
3. **Agent.** Behind the latest release, the agent's `status` shows the board as `update_available`
   with the `update` action.
