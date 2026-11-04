//! Klipper MCU status reporting.

use std::process::ExitCode;

use aldis::checkout::RefreshResult;
use aldis::eligibility::{
    CheckoutRevision, Eligibility, RevisionStatus, TargetVersions, assess_mcu,
};
use aldis::moonraker::{McuInventory, McuTransport, MoonrakerAdapter, MoonrakerPort};
use anyhow::Context;

use crate::cli::ConnectionArgs;
use crate::fail;

pub(crate) fn status(arguments: ConnectionArgs) -> ExitCode {
    let source = arguments
        .klipper_source
        .unwrap_or_else(crate::default_klipper_source);
    let moonraker = MoonrakerAdapter::new(&arguments.moonraker.moonraker);
    let revision = aldis::checkout::revision(&source).unwrap_or(CheckoutRevision::Indeterminate);
    tracing::debug!(?revision, "checkout revision resolved");
    let versions = TargetVersions::klipper(revision);
    match status_report(
        &arguments.moonraker.moonraker,
        &moonraker,
        &aldis::foci::GithubReleases::new(),
        versions,
        &source,
        arguments.beacon_source.as_deref(),
    ) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => fail(format!("{error:#}")),
    }
}

fn status_report(
    url: &str,
    moonraker: &(impl MoonrakerPort + aldis::moonraker::HostPort),
    releases: &impl aldis::foci::ReleasePort,
    versions: TargetVersions,
    source: &std::path::Path,
    beacon_source: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    tracing::info!("discovering MCUs from Moonraker");
    let mut inventory =
        crate::discovery::discover_mcus_with_retry(moonraker).context("failed to discover MCUs")?;
    crate::discovery::probe_unreported(url, moonraker, &mut inventory);
    let versions = versions
        .with_foci_release(crate::discovery::foci_release(&inventory, releases))
        .with_beacon_targets(aldis::beacon::targets_for(
            &inventory,
            beacon_source,
            Some(source),
        ));
    Ok(format_status(source, &versions, &inventory, None))
}

pub(crate) fn format_status(
    source: &std::path::Path,
    versions: &TargetVersions,
    inventory: &McuInventory,
    refreshed: Option<&RefreshResult>,
) -> String {
    let revision = refreshed.map_or_else(|| versions.klipper.label().to_owned(), refresh_label);
    let mut output = format!(
        "Klipper source:    {}\nCheckout revision: {}\n",
        source.display(),
        revision,
    );
    if inventory.mcus.iter().any(aldis::foci::is_foci) {
        output.push_str(&format!(
            "FOCI release:      {}\n",
            versions
                .foci
                .as_ref()
                .map_or("unknown", |release| release.tag.as_str())
        ));
    }
    if inventory.mcus.iter().any(aldis::beacon::is_beacon) {
        output.push_str(&format!(
            "Beacon firmware:   {}\n",
            versions.beacon.checkout.as_ref().map_or_else(
                || "no beacon_klipper checkout found".to_owned(),
                |checkout| checkout.display().to_string()
            )
        ));
    }
    for mcu in &inventory.mcus {
        let status = assess_mcu(mcu, versions);
        output.push_str(&format!("\n{}\n", mcu.name));
        for (label, value) in [
            ("firmware:", firmware_label(mcu, &status.eligibility)),
            (
                "version:",
                mcu.version.as_deref().unwrap_or("unknown").to_owned(),
            ),
            ("model:", mcu.mcu.clone()),
            ("connection:", connection_label(mcu.transport.as_ref())),
            (
                "supported:",
                supported_label(mcu, &status.eligibility, versions),
            ),
            ("needs update:", update_label(status.revision)),
        ] {
            output.push_str(&format!("  {label:<13} {value}\n"));
        }
    }
    for unreported in &inventory.unreported {
        output.push_str(&format!("\n{}\n", unreported.name));
        for (label, value) in [
            (
                "connection:",
                connection_label(unreported.transport.as_ref()),
            ),
            ("supported:", format!("no ({})", unreported.reason)),
        ] {
            output.push_str(&format!("  {label:<13} {value}\n"));
        }
    }
    output
}

fn firmware_label(mcu: &aldis::moonraker::Mcu, eligibility: &Eligibility) -> String {
    match eligibility {
        Eligibility::ExternallyManaged(name) => return name.clone(),
        Eligibility::Beacon(_) => return "Beacon".to_owned(),
        _ => {}
    }
    if let Some(app) = mcu.app.as_deref().filter(|app| !app.trim().is_empty()) {
        app.to_owned()
    } else if !mcu.kconfig.trim().is_empty() {
        "Klipper".to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn connection_label(transport: Option<&McuTransport>) -> String {
    match transport {
        Some(McuTransport::Serial { device }) => format!("serial ({device})"),
        Some(McuTransport::Can { interface, uuid }) => format!("CAN ({interface}, {uuid:012x})"),
        None => "not configured".to_owned(),
    }
}

fn supported_label(
    mcu: &aldis::moonraker::Mcu,
    eligibility: &Eligibility,
    versions: &TargetVersions,
) -> String {
    match eligibility {
        Eligibility::Beacon(_) => match versions.beacon.mcus.get(&mcu.name) {
            Some(Ok(target)) => format!(
                "yes ({} holds {})",
                target.path.file_name().map_or_else(
                    || target.path.display().to_string(),
                    |name| name.to_string_lossy().into_owned()
                ),
                target.version.reported()
            ),
            Some(Err(reason)) => format!("yes (no Beacon image known: {reason})"),
            None => "yes (no Beacon image known)".to_owned(),
        },
        Eligibility::Eligible => "yes".to_owned(),
        Eligibility::Foci(firmware) => format!(
            "yes (FOCI {} {} release builds)",
            firmware.board.crate_name(),
            firmware.flavor.label()
        ),
        Eligibility::ExternallyManaged(app) => format!("no (third-party firmware: {app})"),
        Eligibility::Unsupported(reason) => format!("no ({reason})"),
        Eligibility::UnsupportedMcu(family) => {
            format!("no ({family} boards have no bootloader aldis can flash)")
        }
    }
}

fn update_label(revision: Option<RevisionStatus>) -> String {
    match revision {
        Some(RevisionStatus::Current) => "no".to_owned(),
        Some(RevisionStatus::UpdateRequired) => "yes".to_owned(),
        Some(RevisionStatus::Indeterminate) => "unknown".to_owned(),
        None => "n/a".to_owned(),
    }
}

pub(crate) fn refresh_label(refreshed: &RefreshResult) -> String {
    let suffix = if refreshed.commits_advanced == 1 {
        "commit"
    } else {
        "commits"
    };
    format!(
        "{} -> {} ({} {suffix})",
        refreshed.before.label(),
        refreshed.after.label(),
        refreshed.commits_advanced,
    )
}

#[cfg(test)]
mod tests {
    use super::{format_status, status_report};
    use aldis::checkout::RefreshResult;
    use aldis::eligibility::{CheckoutRevision, TargetVersions};
    use aldis::foci::{Release, ReleaseError, ReleasePort};
    use aldis::moonraker::{Mcu, McuInventory, McuTransport, MoonrakerError, MoonrakerPort};

    #[test]
    fn renders_human_readable_klipper_status() {
        let inventory = McuInventory {
            mcus: vec![mcu(
                "mcu",
                "v0.12.0-123-deadbeef",
                Some("/dev/serial/by-id/mcu"),
            )],
            unreported: Vec::new(),
        };

        assert_eq!(
            format_status(
                std::path::Path::new("/home/pi/klipper"),
                &TargetVersions::klipper(CheckoutRevision::Known(
                    "v0.13.0-756-g2d7717e3".to_owned()
                )),
                &inventory,
                None,
            ),
            concat!(
                "Klipper source:    /home/pi/klipper\n",
                "Checkout revision: v0.13.0-756-g2d7717e3\n\n",
                "mcu\n",
                "  firmware:     Klipper\n",
                "  version:      v0.12.0-123-deadbeef\n",
                "  model:        test\n",
                "  connection:   serial (/dev/serial/by-id/mcu)\n",
                "  supported:    yes\n",
                "  needs update: yes\n",
            )
        );
    }

    #[test]
    fn folds_the_unsupported_reason_into_the_supported_line() {
        let inventory = McuInventory {
            mcus: vec![
                Mcu {
                    name: "cartographer".to_owned(),
                    app: Some("Cartographer".to_owned()),
                    version: Some("CARTOGRAPHER V4 6.1.0".to_owned()),
                    mcu: "stm32g431xx".to_owned(),
                    canbus_frequency_hz: None,
                    build_versions: None,
                    transport: None,
                    kconfig: String::new(),
                },
                Mcu {
                    name: "old-mcu".to_owned(),
                    app: Some("Klipper".to_owned()),
                    version: Some("v0.12.0-500-gdeadbeef".to_owned()),
                    mcu: "rp2040".to_owned(),
                    canbus_frequency_hz: None,
                    build_versions: None,
                    transport: None,
                    kconfig: String::new(),
                },
            ],
            unreported: Vec::new(),
        };

        let output = format_status(
            std::path::Path::new("/home/pi/klipper"),
            &TargetVersions::klipper(CheckoutRevision::Known("v0.13.0-753-g8c29c0a8e".to_owned())),
            &inventory,
            None,
        );

        assert!(output.contains("  firmware:     Cartographer\n"));
        assert!(output.contains("  supported:    no (third-party firmware: Cartographer)\n"));
        assert!(output.contains(
            "  supported:    no (Klipper version too old (v0.12.0-500-gdeadbeef < v0.13.0-753-g8c29c0a8e))\n"
        ));
    }

    #[test]
    fn shows_the_beacon_checkout_and_the_image_each_beacon_takes() {
        let beacon = |name: &str| Mcu {
            name: name.to_owned(),
            app: Some(String::new()),
            version: Some("Beacon 2.0.1".to_owned()),
            mcu: "samd21g18a".to_owned(),
            canbus_frequency_hz: None,
            build_versions: None,
            transport: Some(McuTransport::Serial {
                device: "/dev/serial/by-id/usb-Beacon_Beacon_RevH-if00".to_owned(),
            }),
            kconfig: String::new(),
        };
        let inventory = McuInventory {
            mcus: vec![beacon("mcu beacon"), beacon("mcu beacon sensor rear")],
            unreported: Vec::new(),
        };
        let checkout = std::path::PathBuf::from("/home/pi/beacon_klipper");
        let versions = TargetVersions::klipper(CheckoutRevision::Known("v2".to_owned()))
            .with_beacon_targets(aldis::beacon::Targets {
                checkout: Some(checkout.clone()),
                mcus: [
                    (
                        "mcu beacon".to_owned(),
                        Ok(aldis::beacon::Target {
                            revision: "h".to_owned(),
                            path: checkout.join("firmware/revh.dfu"),
                            version: aldis::beacon::Version::from_bcd(0x2010),
                        }),
                    ),
                    (
                        "mcu beacon sensor rear".to_owned(),
                        Err(aldis::beacon::Unavailable::UnknownRevision(
                            "Beacon Classic".to_owned(),
                        )),
                    ),
                ]
                .into(),
            });

        let output = format_status(
            std::path::Path::new("/home/pi/klipper"),
            &versions,
            &inventory,
            None,
        );

        assert!(
            output.contains("Beacon firmware:   /home/pi/beacon_klipper\n"),
            "{output}"
        );
        assert!(
            output.contains(concat!(
                "mcu beacon\n",
                "  firmware:     Beacon\n",
                "  version:      Beacon 2.0.1\n",
                "  model:        samd21g18a\n",
                "  connection:   serial (/dev/serial/by-id/usb-Beacon_Beacon_RevH-if00)\n",
                "  supported:    yes (revh.dfu holds Beacon 2.1.0)\n",
                "  needs update: yes\n",
            )),
            "{output}"
        );
        assert!(
            output.contains(concat!(
                "  supported:    yes (no Beacon image known: its USB product name ",
                "\"Beacon Classic\" names no Beacon revision)\n",
                "  needs update: unknown\n",
            )),
            "{output}"
        );

        let missing = format_status(
            std::path::Path::new("/home/pi/klipper"),
            &TargetVersions::klipper(CheckoutRevision::Known("v2".to_owned())),
            &inventory,
            None,
        );
        assert!(
            missing.contains("Beacon firmware:   no beacon_klipper checkout found\n"),
            "{missing}"
        );
    }

    #[test]
    fn lists_unreported_mcus_with_their_reason() {
        use aldis::moonraker::{UnreportedMcu, UnreportedReason};

        let inventory = McuInventory {
            mcus: Vec::new(),
            unreported: vec![UnreportedMcu {
                name: "mcu xiao".to_owned(),
                transport: None,
                reason: UnreportedReason::NotResponding,
            }],
        };

        let output = format_status(
            std::path::Path::new("/home/pi/klipper"),
            &TargetVersions::klipper(CheckoutRevision::Known("v1".to_owned())),
            &inventory,
            None,
        );

        assert!(output.contains("\nmcu xiao\n"), "{output}");
        assert!(
            output.contains("not responding; run `aldis reboot` or power-cycle the board"),
            "{output}"
        );
    }

    #[test]
    fn includes_refresh_details_in_the_aligned_checkout_field() {
        let refreshed = RefreshResult {
            before: CheckoutRevision::Known("v0.12.0-123-deadbeef".to_owned()),
            after: CheckoutRevision::Known("v0.13.0-756-g2d7717e3".to_owned()),
            advanced: true,
            commits_advanced: 4,
        };

        let output = format_status(
            std::path::Path::new("/home/pi/klipper"),
            &TargetVersions::klipper(refreshed.after.clone()),
            &McuInventory {
                mcus: Vec::new(),
                unreported: Vec::new(),
            },
            Some(&refreshed),
        );

        assert_eq!(
            output,
            concat!(
                "Klipper source:    /home/pi/klipper\n",
                "Checkout revision: v0.12.0-123-deadbeef -> v0.13.0-756-g2d7717e3 (4 commits)\n",
            )
        );
    }

    #[test]
    fn reports_status_from_an_injected_moonraker_and_a_known_revision() {
        let moonraker = FakeMoonraker(Ok(McuInventory {
            mcus: vec![mcu("mcu h723", "v2", None)],
            unreported: Vec::new(),
        }));
        let report = status_report(
            "http://127.0.0.1:7125",
            &moonraker,
            &NoReleases,
            TargetVersions::klipper(CheckoutRevision::Known("v2".to_owned())),
            std::path::Path::new("/klipper"),
            None,
        )
        .expect("status report should succeed");

        assert!(report.contains("mcu h723"));
        assert!(report.contains("v2"));
    }

    #[test]
    fn reports_the_moonraker_error_when_discovery_fails() {
        let moonraker = FakeMoonraker(Err("no MCU objects were reported".to_owned()));
        let error = status_report(
            "http://127.0.0.1:7125",
            &moonraker,
            &NoReleases,
            TargetVersions::indeterminate(),
            std::path::Path::new("/klipper"),
            None,
        )
        .expect_err("status report should fail");

        assert!(format!("{error:#}").contains("no MCU objects were reported"));
    }

    #[test]
    fn compares_foci_mcus_with_the_latest_foci_release() {
        let mut foci = mcu("mcu ouroboros", "v0.4.0-0-g0fb68e1b", None);
        foci.app = Some("FOCI".to_owned());
        foci.mcu = "stm32h723xx".to_owned();
        foci.kconfig = String::new();
        foci.build_versions = Some("rust=1.98.1 features=none board=0fb68e1b".to_owned());
        let moonraker = FakeMoonraker(Ok(McuInventory {
            mcus: vec![mcu("mcu", "v2", None), foci],
            unreported: Vec::new(),
        }));
        let report = |releases: &dyn Fn() -> Result<Release, ReleaseError>| {
            status_report(
                "http://127.0.0.1:7125",
                &moonraker,
                &FakeReleases(releases),
                TargetVersions::klipper(CheckoutRevision::Known("v2".to_owned())),
                std::path::Path::new("/klipper"),
                None,
            )
            .expect("status report should succeed")
        };

        let found = report(&|| Ok(Release::from_tag("v0.4.1").unwrap()));
        assert!(found.contains("FOCI release:      v0.4.1\n"), "{found}");
        assert!(
            found.contains(
                "mcu ouroboros\n  firmware:     FOCI\n  version:      v0.4.0-0-g0fb68e1b\n"
            ),
            "{found}"
        );
        assert!(
            found.contains(
                "  supported:    yes (FOCI ouroboros-fw prod release builds)\n  needs update: yes\n"
            ),
            "{found}"
        );

        let unknown = report(&|| Err(ReleaseError::InvalidTag("nightly".to_owned())));
        assert!(
            unknown.contains("FOCI release:      unknown\n"),
            "{unknown}"
        );
        assert!(unknown.contains("  needs update: unknown\n"), "{unknown}");
    }

    #[test]
    fn neither_looks_up_nor_shows_a_foci_release_without_a_foci_mcu() {
        let moonraker = FakeMoonraker(Ok(McuInventory {
            mcus: vec![mcu("mcu", "v2", None)],
            unreported: Vec::new(),
        }));
        let report = status_report(
            "http://127.0.0.1:7125",
            &moonraker,
            &NoReleases,
            TargetVersions::klipper(CheckoutRevision::Known("v2".to_owned())),
            std::path::Path::new("/klipper"),
            None,
        )
        .expect("status report should succeed");

        assert!(!report.contains("FOCI"), "{report}");
    }

    /// A release source that must not be asked.
    struct NoReleases;

    impl ReleasePort for NoReleases {
        fn latest_release(&self) -> Result<Release, ReleaseError> {
            panic!("no FOCI MCU, so no release lookup");
        }

        fn download_firmware(&self, _: &Release, _: &str) -> Result<Vec<u8>, ReleaseError> {
            panic!("status never downloads firmware");
        }
    }

    struct FakeReleases<'a>(&'a dyn Fn() -> Result<Release, ReleaseError>);

    impl ReleasePort for FakeReleases<'_> {
        fn latest_release(&self) -> Result<Release, ReleaseError> {
            (self.0)()
        }

        fn download_firmware(&self, _: &Release, _: &str) -> Result<Vec<u8>, ReleaseError> {
            panic!("status never downloads firmware");
        }
    }

    struct FakeMoonraker(Result<McuInventory, String>);

    impl MoonrakerPort for FakeMoonraker {
        fn discover_mcus(&self) -> Result<McuInventory, MoonrakerError> {
            self.0.clone().map_err(MoonrakerError::InvalidResponse)
        }
    }

    impl aldis::moonraker::HostPort for FakeMoonraker {
        fn klipper_unit(&self) -> Result<Option<String>, MoonrakerError> {
            Ok(None)
        }

        fn host_info(&self) -> Result<aldis::moonraker::HostInfo, MoonrakerError> {
            Ok(aldis::moonraker::HostInfo {
                state: aldis::moonraker::KlippyState::Ready,
                state_message: String::new(),
                software_version: None,
                klipper_path: None,
            })
        }

        fn logs_root(&self) -> Result<Option<std::path::PathBuf>, MoonrakerError> {
            Ok(None)
        }
    }

    fn mcu(name: &str, version: &str, serial: Option<&str>) -> Mcu {
        Mcu {
            name: name.to_owned(),
            app: None,
            version: Some(version.to_owned()),
            mcu: "test".to_owned(),
            canbus_frequency_hz: None,
            build_versions: None,
            transport: serial.map(|device| McuTransport::Serial {
                device: device.to_owned(),
            }),
            kconfig: "CONFIG_TEST=y\n".to_owned(),
        }
    }
}
