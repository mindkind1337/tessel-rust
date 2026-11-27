use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenLinksTarget {
    #[default]
    Builtin,
    System,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserPreferences {
    pub zoom_half_steps: i8,
    pub open_links_in: OpenLinksTarget,
    pub separate_agent_session: bool,
}

impl BrowserPreferences {
    pub fn zoom_factor(steps: i8) -> f64 {
        1.2_f64.powf(f64::from(steps.clamp(-6, 10)) / 2.0)
    }
    pub fn zoom_percent(steps: i8) -> i32 {
        (Self::zoom_factor(steps) * 100.0).round() as i32
    }
}

impl Default for BrowserPreferences {
    fn default() -> Self {
        Self {
            zoom_half_steps: -1,
            open_links_in: OpenLinksTarget::Builtin,
            separate_agent_session: true,
        }
    }
}
