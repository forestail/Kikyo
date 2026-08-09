use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetPlatform {
    #[default]
    Windows,
    Macos,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExclusionScope {
    #[default]
    Application,
    Window,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForegroundTarget {
    pub platform: TargetPlatform,
    pub display_name: String,
    pub app_id: String,
    pub executable_path: String,
    pub process_name: String,
    pub window_title: String,
    pub window_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExcludedTarget {
    pub id: String,
    pub enabled: bool,
    pub scope: ExclusionScope,
    pub platform: TargetPlatform,
    pub display_name: String,
    pub app_id: String,
    pub executable_path: String,
    pub process_name: String,
    pub window_title_contains: String,
    pub window_class: String,
}

impl Default for ExcludedTarget {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: true,
            scope: ExclusionScope::Application,
            platform: TargetPlatform::Windows,
            display_name: String::new(),
            app_id: String::new(),
            executable_path: String::new(),
            process_name: String::new(),
            window_title_contains: String::new(),
            window_class: String::new(),
        }
    }
}

impl ExcludedTarget {
    pub fn from_foreground(id: String, target: &ForegroundTarget, scope: ExclusionScope) -> Self {
        Self {
            id,
            enabled: true,
            scope,
            platform: target.platform,
            display_name: target.display_name.clone(),
            app_id: target.app_id.clone(),
            executable_path: target.executable_path.clone(),
            process_name: target.process_name.clone(),
            window_title_contains: if scope == ExclusionScope::Window {
                target.window_title.clone()
            } else {
                String::new()
            },
            window_class: if scope == ExclusionScope::Window {
                target.window_class.clone()
            } else {
                String::new()
            },
        }
    }

    pub fn normalize(&mut self) {
        self.id = self.id.trim().to_string();
        self.display_name = self.display_name.trim().to_string();
        self.app_id = self.app_id.trim().to_string();
        self.executable_path = normalize_path(&self.executable_path, self.platform);
        self.process_name = self.process_name.trim().to_string();
        self.window_title_contains = self.window_title_contains.trim().to_string();
        self.window_class = self.window_class.trim().to_string();
    }

    pub fn is_valid(&self) -> bool {
        let has_application_identity = !self.app_id.trim().is_empty()
            || !self.executable_path.trim().is_empty()
            || !self.process_name.trim().is_empty();
        if !has_application_identity {
            return false;
        }
        self.scope != ExclusionScope::Window
            || !self.window_title_contains.trim().is_empty()
            || !self.window_class.trim().is_empty()
    }

    pub fn matches(&self, target: &ForegroundTarget) -> bool {
        if !self.enabled || self.platform != target.platform || !self.application_matches(target) {
            return false;
        }
        if self.scope == ExclusionScope::Application {
            return true;
        }

        let class_matches = self.window_class.trim().is_empty()
            || text_eq(&self.window_class, &target.window_class, self.platform);
        let title_matches = self.window_title_contains.trim().is_empty()
            || text_contains(
                &target.window_title,
                &self.window_title_contains,
                self.platform,
            );
        class_matches && title_matches
    }

    fn application_matches(&self, target: &ForegroundTarget) -> bool {
        if !self.app_id.trim().is_empty() {
            return text_eq(&self.app_id, &target.app_id, self.platform);
        }
        if !self.executable_path.trim().is_empty() {
            return normalize_path(&self.executable_path, self.platform)
                == normalize_path(&target.executable_path, self.platform);
        }
        !self.process_name.trim().is_empty()
            && text_eq(&self.process_name, &target.process_name, self.platform)
    }
}

pub fn is_target_excluded(rules: &[ExcludedTarget], target: &ForegroundTarget) -> bool {
    rules.iter().any(|rule| rule.matches(target))
}

fn normalize_path(value: &str, platform: TargetPlatform) -> String {
    let trimmed = value.trim();
    match platform {
        TargetPlatform::Windows => trimmed.replace('/', "\\").to_lowercase(),
        TargetPlatform::Macos => trimmed.to_string(),
    }
}

fn text_eq(left: &str, right: &str, platform: TargetPlatform) -> bool {
    match platform {
        TargetPlatform::Windows => left.eq_ignore_ascii_case(right),
        TargetPlatform::Macos => left == right,
    }
}

fn text_contains(haystack: &str, needle: &str, platform: TargetPlatform) -> bool {
    match platform {
        TargetPlatform::Windows => haystack.to_lowercase().contains(&needle.to_lowercase()),
        TargetPlatform::Macos => haystack.contains(needle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_target() -> ForegroundTarget {
        ForegroundTarget {
            platform: TargetPlatform::Windows,
            display_name: "Editor".to_string(),
            executable_path: r"C:\Program Files\Editor\editor.exe".to_string(),
            process_name: "editor.exe".to_string(),
            window_title: "notes.txt - Editor".to_string(),
            window_class: "EditorWindow".to_string(),
            ..ForegroundTarget::default()
        }
    }

    #[test]
    fn application_rule_matches_windows_path_case_and_slashes() {
        let mut rule = ExcludedTarget::default();
        rule.executable_path = "c:/program files/editor/EDITOR.EXE".to_string();
        assert!(rule.matches(&windows_target()));
    }

    #[test]
    fn window_rule_requires_all_configured_window_fields() {
        let mut rule = ExcludedTarget::default();
        rule.scope = ExclusionScope::Window;
        rule.process_name = "EDITOR.EXE".to_string();
        rule.window_class = "EditorWindow".to_string();
        rule.window_title_contains = "notes.txt".to_string();
        assert!(rule.matches(&windows_target()));

        rule.window_title_contains = "other.txt".to_string();
        assert!(!rule.matches(&windows_target()));
    }

    #[test]
    fn disabled_and_invalid_rules_do_not_match_or_validate() {
        let mut disabled = ExcludedTarget::default();
        disabled.process_name = "editor.exe".to_string();
        disabled.enabled = false;
        assert!(!disabled.matches(&windows_target()));

        let mut invalid_window = ExcludedTarget::default();
        invalid_window.scope = ExclusionScope::Window;
        invalid_window.process_name = "editor.exe".to_string();
        assert!(!invalid_window.is_valid());
    }
}
