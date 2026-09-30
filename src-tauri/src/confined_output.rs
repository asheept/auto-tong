use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, File, OpenOptions};

use crate::import_path::safe_relative;

/// All archive-controlled names are resolved from an open directory handle.
/// Parent handles are opened without following symlinks or Windows junctions.
pub struct ConfinedOutput {
    root: Dir,
}

impl ConfinedOutput {
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = Dir::open_ambient_dir(root, cap_std::ambient_authority())
            .map_err(|error| format!("출력 루트 열기 실패: {error}"))?;
        Ok(Self { root })
    }

    fn dir_for(&self, relative: &Path) -> Result<Dir, String> {
        let mut dir = self.root.try_clone().map_err(|error| error.to_string())?;
        for component in relative.components() {
            let name = component.as_os_str();
            match dir.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(format!("출력 디렉터리 생성 실패: {error}")),
            }
            dir = dir
                .open_dir_nofollow(name)
                .map_err(|error| format!("링크 또는 junction 출력 경로 거부: {error}"))?;
        }
        Ok(dir)
    }

    pub fn create_dir(&self, raw: &str) -> Result<(), String> {
        let relative = safe_relative(raw)?;
        self.dir_for(&relative).map(|_| ())
    }

    pub fn subdir(&self, raw: &str) -> Result<Self, String> {
        let relative = safe_relative(raw)?;
        Ok(Self {
            root: self.dir_for(&relative)?,
        })
    }

    pub fn write_file(
        &self,
        raw: &str,
        contents: impl AsRef<[u8]>,
        replace_regular: bool,
    ) -> Result<(), String> {
        let mut file = self.create_file(raw, replace_regular)?;
        file.write_all(contents.as_ref())
            .map_err(|error| format!("출력 파일 쓰기 실패: {error}"))
    }

    pub fn read_to_string(&self, raw: &str) -> Result<Option<String>, String> {
        let Some(mut file) = self.open_file(raw)? else {
            return Ok(None);
        };
        let mut content = String::new();
        use std::io::Read;
        file.read_to_string(&mut content)
            .map_err(|error| error.to_string())?;
        Ok(Some(content))
    }

    fn parent(&self, raw: &str) -> Result<(Dir, PathBuf), String> {
        let relative = safe_relative(raw)?;
        let name = relative.file_name().ok_or("출력 파일 이름 없음")?.into();
        let parent = self.dir_for(relative.parent().unwrap_or(Path::new("")))?;
        Ok((parent, name))
    }

    pub fn create_file(&self, raw: &str, replace_regular: bool) -> Result<File, String> {
        let (parent, name) = self.parent(raw)?;
        match parent.symlink_metadata(&name) {
            Ok(metadata) if replace_regular && metadata.file_type().is_file() => {
                parent
                    .remove_file(&name)
                    .map_err(|error| format!("기존 출력 파일 제거 실패: {error}"))?;
            }
            Ok(_) => return Err(format!("중복 또는 링크 출력 경로: {raw}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("출력 파일 확인 실패: {error}")),
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        parent
            .open_with(&name, &options)
            .map_err(|error| format!("출력 파일 생성 실패 {raw}: {error}"))
    }

    pub fn open_file(&self, raw: &str) -> Result<Option<File>, String> {
        let (parent, name) = self.parent(raw)?;
        match parent.symlink_metadata(&name) {
            Ok(metadata) if metadata.file_type().is_file() => parent
                .open(&name)
                .map(Some)
                .map_err(|error| error.to_string()),
            Ok(_) => Err(format!("링크 또는 특수 출력 경로: {raw}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn remove_file(&self, raw: &str) -> Result<(), String> {
        let (parent, name) = self.parent(raw)?;
        let metadata = parent
            .symlink_metadata(&name)
            .map_err(|error| error.to_string())?;
        if !metadata.file_type().is_file() {
            return Err(format!("링크 또는 특수 출력 경로: {raw}"));
        }
        parent.remove_file(name).map_err(|error| error.to_string())
    }

    pub fn rename_file(&self, from: &str, to: &str) -> Result<(), String> {
        let (source_parent, source_name) = self.parent(from)?;
        let (target_parent, target_name) = self.parent(to)?;
        if target_parent.symlink_metadata(&target_name).is_ok() {
            return Err(format!("중복 또는 링크 출력 경로: {to}"));
        }
        source_parent
            .rename(source_name, &target_parent, target_name)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_link_after_directory_was_checked() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("stage");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let output = ConfinedOutput::open(&root).unwrap();
        output.create_dir("mods").unwrap();
        std::fs::remove_dir(root.join("mods")).unwrap();
        let link = root.join("mods");
        #[cfg(target_os = "windows")]
        let linked = {
            std::os::windows::fs::symlink_dir(&outside, &link).is_ok()
                || std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(&link)
                    .arg(&outside)
                    .output()
                    .is_ok_and(|result| result.status.success())
        };
        #[cfg(not(target_os = "windows"))]
        let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
        if !linked {
            eprintln!("symlink/junction 생성 권한이 없어 해당 fixture를 건너뜁니다");
            return;
        }
        assert!(output.create_file("mods/escape.txt", false).is_err());
        assert!(!outside.join("escape.txt").exists());
    }

    #[test]
    fn rejects_link_leaf_without_touching_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("stage");
        std::fs::create_dir(&root).unwrap();
        let outside = temp.path().join("outside.txt");
        std::fs::write(&outside, b"original").unwrap();
        let link = root.join("pack.txt");
        #[cfg(target_os = "windows")]
        let linked = std::os::windows::fs::symlink_file(&outside, &link).is_ok();
        #[cfg(not(target_os = "windows"))]
        let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
        if !linked {
            eprintln!("file symlink 생성 권한이 없어 해당 fixture를 건너뜁니다");
            return;
        }
        let output = ConfinedOutput::open(&root).unwrap();
        assert!(output.create_file("pack.txt", true).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"original");
    }
}
