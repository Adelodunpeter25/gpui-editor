//! Theme system and color mappings for syntax highlighting.

pub use lumis::themes::Theme;

/// Theme identifier string wrapper or preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreset {
    Dracula,
    OneDark,
    GitHubDark,
    GitHubLight,
    CatppuccinMocha,
    Nord,
    TokyoNightNight,
    SolarizedDark,
    SolarizedLight,
    MonokaiPro,
    VsCodeDark,
    VsCodeLight,
}

impl ThemePreset {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dracula => "dracula",
            Self::OneDark => "onedark",
            Self::GitHubDark => "github_dark",
            Self::GitHubLight => "github_light",
            Self::CatppuccinMocha => "catppuccin_mocha",
            Self::Nord => "nord",
            Self::TokyoNightNight => "tokyonight_night",
            Self::SolarizedDark => "solarized_autumn_dark",
            Self::SolarizedLight => "solarized_autumn_light",
            Self::MonokaiPro => "monokai_pro_dark",
            Self::VsCodeDark => "vscode_dark",
            Self::VsCodeLight => "vscode_light",
        }
    }

    pub fn to_theme(&self) -> Option<Theme> {
        lumis::themes::get(self.name()).ok()
    }
}

/// Retrieve a Lumis Theme by name.
pub fn get_theme(name: &str) -> Option<Theme> {
    lumis::themes::get(name).ok()
}
