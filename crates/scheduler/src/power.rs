//! Battery and metered-connection state, and the hold policy.

use velox_types::{PowerHold, PowerSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerState {
    /// Running on battery (not on AC power).
    pub on_battery: bool,
    /// Battery charge, when known.
    pub battery_percent: Option<u8>,
    /// The active internet connection is metered (Windows only).
    pub metered: bool,
}

/// Source of the current power state (replaceable in tests).
pub trait PowerSource: Send + Sync {
    fn state(&self) -> PowerState;
}

/// Whether the settings hold automatic queue processing back.
pub fn hold(settings: &PowerSettings, s: &PowerState) -> Option<PowerHold> {
    if settings.pause_on_low_battery
        && s.on_battery
        && s.battery_percent
            .is_some_and(|p| p < settings.battery_threshold)
    {
        return Some(PowerHold::LowBattery);
    }
    if settings.pause_on_metered && s.metered {
        return Some(PowerHold::Metered);
    }
    None
}

/// The operating system's power state.
pub struct SystemPower;

impl PowerSource for SystemPower {
    fn state(&self) -> PowerState {
        imp::state()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::PowerState;

    pub fn state() -> PowerState {
        parse_sysfs(std::path::Path::new("/sys/class/power_supply"))
    }

    /// Reads `type`, `status`, `capacity` and `online` of every supply.
    pub(super) fn parse_sysfs(root: &std::path::Path) -> PowerState {
        let read = |p: std::path::PathBuf| {
            std::fs::read_to_string(p)
                .map(|s| s.trim().to_string())
                .ok()
        };
        let mut st = PowerState::default();
        let mut mains_online = false;
        let mut battery = false;
        let Ok(entries) = std::fs::read_dir(root) else {
            return st;
        };
        for e in entries.flatten() {
            let dir = e.path();
            match read(dir.join("type")).as_deref() {
                Some("Mains") | Some("USB") => {
                    mains_online |= read(dir.join("online")).as_deref() == Some("1")
                }
                Some("Battery") => {
                    // Peripheral batteries (mice, keyboards) report scope=Device.
                    if read(dir.join("scope")).as_deref() == Some("Device") {
                        continue;
                    }
                    battery = true;
                    if let Some(c) = read(dir.join("capacity")).and_then(|c| c.parse::<u8>().ok()) {
                        st.battery_percent = Some(st.battery_percent.map_or(c, |p| p.min(c)));
                    }
                    if read(dir.join("status")).as_deref() == Some("Discharging") {
                        st.on_battery = true;
                    }
                }
                _ => {}
            }
        }
        if battery && !mains_online && st.battery_percent.is_some() {
            st.on_battery = true;
        }
        st
    }
}

#[cfg(windows)]
mod imp {
    use super::PowerState;
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    pub fn state() -> PowerState {
        let mut st = PowerState::default();
        // SAFETY: GetSystemPowerStatus fills the plain-data struct we own.
        let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { GetSystemPowerStatus(&mut s) } != 0 {
            let has_battery = s.BatteryFlag != 128 && s.BatteryFlag != 255;
            st.on_battery = has_battery && s.ACLineStatus == 0;
            if has_battery && s.BatteryLifePercent <= 100 {
                st.battery_percent = Some(s.BatteryLifePercent);
            }
        }
        st.metered = metered().unwrap_or(false);
        st
    }

    /// Cost type of the internet connection profile (WinRT).
    fn metered() -> windows::core::Result<bool> {
        use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        // The calling thread may not have joined an apartment yet; an error
        // only means it already has one.
        // SAFETY: plain COM initialisation of the current thread.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let Ok(profile) = NetworkInformation::GetInternetConnectionProfile() else {
            return Ok(false);
        };
        let cost = profile.GetConnectionCost()?;
        let kind = cost.NetworkCostType()?;
        Ok(kind == NetworkCostType::Fixed
            || kind == NetworkCostType::Variable
            || cost.Roaming()?
            || cost.OverDataLimit()?)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::PowerState;

    pub fn state() -> PowerState {
        let Ok(out) = std::process::Command::new("/usr/bin/pmset")
            .args(["-g", "batt"])
            .output()
        else {
            return PowerState::default();
        };
        super::parse_pmset(&String::from_utf8_lossy(&out.stdout))
    }
}

/// Parses `pmset -g batt` (macOS).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_pmset(text: &str) -> PowerState {
    let mut st = PowerState {
        on_battery: text.contains("'Battery Power'"),
        ..Default::default()
    };
    st.battery_percent = text
        .split(|c: char| c.is_whitespace() || c == ';')
        .find_map(|w| w.strip_suffix('%')?.parse::<u8>().ok());
    st
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod imp {
    pub fn state() -> super::PowerState {
        super::PowerState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_policy() {
        let mut s = PowerSettings {
            pause_on_low_battery: true,
            battery_threshold: 20,
            pause_on_metered: false,
        };
        let low = PowerState {
            on_battery: true,
            battery_percent: Some(15),
            metered: true,
        };
        assert_eq!(hold(&s, &low), Some(PowerHold::LowBattery));
        assert_eq!(
            hold(
                &s,
                &PowerState {
                    on_battery: false,
                    ..low
                }
            ),
            None,
            "charging"
        );
        assert_eq!(
            hold(
                &s,
                &PowerState {
                    battery_percent: Some(50),
                    ..low
                }
            ),
            None
        );
        s.pause_on_metered = true;
        assert_eq!(
            hold(
                &s,
                &PowerState {
                    battery_percent: Some(50),
                    ..low
                }
            ),
            Some(PowerHold::Metered)
        );
        s.pause_on_low_battery = false;
        s.pause_on_metered = false;
        assert_eq!(hold(&s, &low), None);
    }

    #[test]
    fn macos_pmset() {
        let on_battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1234)\t15%; discharging; 0:40 remaining present: true\n";
        assert_eq!(
            parse_pmset(on_battery),
            PowerState {
                on_battery: true,
                battery_percent: Some(15),
                metered: false
            }
        );
        let ac = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1234)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(
            parse_pmset(ac),
            PowerState {
                on_battery: false,
                battery_percent: Some(100),
                metered: false
            }
        );
        assert_eq!(
            parse_pmset("Now drawing from 'AC Power'\n"),
            PowerState::default()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        let mk = |name: &str, files: &[(&str, &str)]| {
            let d = dir.path().join(name);
            std::fs::create_dir_all(&d).unwrap();
            for (f, v) in files {
                std::fs::write(d.join(f), format!("{v}\n")).unwrap();
            }
        };
        mk("AC", &[("type", "Mains"), ("online", "0")]);
        mk(
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("capacity", "17"),
            ],
        );
        mk(
            "hidpp_battery_0",
            &[("type", "Battery"), ("scope", "Device"), ("capacity", "5")],
        );
        let st = imp::parse_sysfs(dir.path());
        assert_eq!(
            st,
            PowerState {
                on_battery: true,
                battery_percent: Some(17),
                metered: false
            }
        );
        mk("AC", &[("type", "Mains"), ("online", "1")]);
        mk(
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Charging"),
                ("capacity", "17"),
            ],
        );
        assert!(!imp::parse_sysfs(dir.path()).on_battery);
        // A desktop without batteries.
        assert_eq!(
            imp::parse_sysfs(&dir.path().join("missing")),
            PowerState::default()
        );
    }
}
