use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    Release,
    Beta,
    Alpha,
}

impl Default for ReleaseChannel {
    fn default() -> Self {
        Self::Release
    }
}

impl ReleaseChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }

    pub fn from_api(value: &str) -> Option<Self> {
        match value {
            "release" => Some(Self::Release),
            "beta" => Some(Self::Beta),
            "alpha" => Some(Self::Alpha),
            _ => None,
        }
    }

    fn fallback_order(self, allow_prerelease_fallback: bool) -> &'static [Self] {
        match (self, allow_prerelease_fallback) {
            (Self::Release, true) => &[Self::Release, Self::Beta, Self::Alpha],
            (Self::Release, false) => &[Self::Release],
            (Self::Beta, _) => &[Self::Beta],
            (Self::Alpha, _) => &[Self::Alpha],
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionChoice {
    pub project_id: String,
    pub title: String,
    pub version_id: String,
    pub version_number: String,
    pub version_type: ReleaseChannel,
    pub requires_confirmation: bool,
}

pub fn choose<'a, T>(
    versions: &'a [T],
    requested: ReleaseChannel,
    allow_prerelease_fallback: bool,
    version_type: impl Fn(&T) -> &str,
    compatible: impl Fn(&T) -> bool,
) -> Option<(&'a T, ReleaseChannel, bool)> {
    requested
        .fallback_order(allow_prerelease_fallback)
        .iter()
        .find_map(|channel| {
            versions
                .iter()
                .find(|version| {
                    compatible(version)
                        && ReleaseChannel::from_api(version_type(version)) == Some(*channel)
                })
                .map(|version| (version, *channel, *channel != requested))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_selection_falls_back_in_safe_order() {
        let versions = ["alpha", "beta"];
        let (_, channel, confirmation) = choose(
            &versions,
            ReleaseChannel::Release,
            true,
            |value| *value,
            |_| true,
        )
        .unwrap();
        assert_eq!(channel, ReleaseChannel::Beta);
        assert!(confirmation);
    }

    #[test]
    fn explicit_beta_does_not_silently_select_alpha() {
        let versions = ["release", "alpha"];
        assert!(
            choose(
                &versions,
                ReleaseChannel::Beta,
                true,
                |value| *value,
                |_| true,
            )
            .is_none()
        );
    }
}
