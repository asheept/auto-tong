use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct OutputNames(HashSet<String>);

impl OutputNames {
    pub fn insert(&mut self, raw: &str) -> Result<(), String> {
        let path = safe_relative(raw)?;
        let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
        if !self.0.insert(normalized) {
            return Err(format!("정규화 후 중복 출력 경로: {raw}"));
        }
        Ok(())
    }
}

/// Archive and manifest paths are interpreted with Windows rules on every host.
/// Valid names are relative components only; callers choose the trusted root.
pub fn safe_relative(raw: &str) -> Result<PathBuf, String> {
    let normalized = raw.replace('\\', "/");
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() || normalized.starts_with('/') {
        return Err(format!("허용되지 않는 출력 경로: {raw}"));
    }

    let mut relative = PathBuf::new();
    for component in trimmed.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with([' ', '.'])
            || component.contains(':')
            || component
                .chars()
                .any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
            || component.chars().any(char::is_control)
        {
            return Err(format!("허용되지 않는 출력 경로: {raw}"));
        }

        let stem = component.split('.').next().unwrap_or("");
        let upper = stem.to_ascii_uppercase();
        let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (upper.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && upper.as_bytes()[3].is_ascii_digit()
                && upper.as_bytes()[3] != b'0');
        if reserved {
            return Err(format!("Windows 예약 파일명: {raw}"));
        }
        relative.push(component);
    }
    Ok(relative)
}

pub fn safe_join(root: &Path, raw: &str) -> Result<PathBuf, String> {
    Ok(root.join(safe_relative(raw)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_windows_and_parent_escape() {
        for name in [
            "../out",
            "a/../out",
            r"\out",
            "/out",
            r"C:\out",
            "C:out",
            r"\\server\share\file",
            r"\\?\C:\out",
            "file.txt:stream",
            "a//b",
            "a/./b",
            "CON",
            "con.txt",
            "LPT1.txt",
            "COM9",
            "a.",
            "a ",
            "bad?.jar",
            "bad<name>.jar",
        ] {
            assert!(safe_relative(name).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn keeps_korean_and_nested_paths() {
        let root = Path::new(r"C:\instances\example");
        assert_eq!(
            safe_join(root, r"모드\하위\파일.jar").unwrap(),
            root.join("모드").join("하위").join("파일.jar")
        );
        assert_eq!(
            safe_relative(".minecraft/config/").unwrap(),
            Path::new(".minecraft/config")
        );
    }

    #[test]
    fn normalized_names_reject_windows_case_and_separator_aliases() {
        let mut names = OutputNames::default();
        names.insert("mods/Foo.jar").unwrap();
        assert!(names.insert("MODS\\foo.jar").is_err());
        names.insert("mods/한글.jar").unwrap();
    }
}
