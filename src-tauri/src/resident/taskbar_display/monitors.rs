//! Taskbar display identities are independent of desktop ordering and coordinates.
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: String,
    pub name: String,
    pub number: u32,
    pub primary: bool,
    pub has_taskbar: bool,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalogue {
    pub schema_version: u32,
    pub monitors: Vec<Monitor>,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fallback {
    None,
    Disconnected,
    NoTaskbar,
}

#[cfg(any(windows, test))]
pub(super) fn select(monitors: &[Monitor], requested: Option<&str>) -> Option<(usize, Fallback)> {
    let fallback = if let Some(id) = requested {
        match monitors.iter().position(|monitor| monitor.id == id) {
            Some(index) if monitors[index].has_taskbar => return Some((index, Fallback::None)),
            Some(_) => Fallback::NoTaskbar,
            None => Fallback::Disconnected,
        }
    } else {
        Fallback::None
    };
    monitors
        .iter()
        .position(|monitor| monitor.primary && monitor.has_taskbar)
        .map(|index| (index, fallback))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(id: &str, primary: bool, has_taskbar: bool) -> Monitor {
        Monitor {
            id: id.into(),
            name: id.into(),
            number: 1,
            primary,
            has_taskbar,
            width: 1920,
            height: 1080,
        }
    }
    #[test]
    fn selection_survives_reordering_disconnect_and_taskbar_changes() {
        let mut monitors = vec![
            monitor("external", false, true),
            monitor("internal", true, true),
        ];
        assert_eq!(select(&monitors, None), Some((1, Fallback::None)));
        assert_eq!(
            select(&monitors, Some("external")),
            Some((0, Fallback::None))
        );
        monitors.reverse();
        assert_eq!(
            select(&monitors, Some("external")),
            Some((1, Fallback::None))
        );
        monitors[1].has_taskbar = false;
        assert_eq!(
            select(&monitors, Some("external")),
            Some((0, Fallback::NoTaskbar))
        );
        monitors.pop();
        assert_eq!(
            select(&monitors, Some("external")),
            Some((0, Fallback::Disconnected))
        );
        monitors.push(monitor("external", false, true));
        assert_eq!(
            select(&monitors, Some("external")),
            Some((1, Fallback::None))
        );
        monitors[0].has_taskbar = false;
        assert_eq!(select(&monitors, None), None);
        assert_eq!(select(&[], Some("external")), None);
    }
}
