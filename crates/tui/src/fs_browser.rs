use std::fs;
use std::path::{Path, PathBuf};

use ratatui::widgets::ListState;

/// A directory listing for the load-list file explorer.
#[derive(Debug, Clone)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_parent: bool,
}

#[derive(Debug, Clone)]
pub struct FileBrowser {
    pub cwd: PathBuf,
    pub showing_drives: bool,
    pub entries: Vec<FsEntry>,
    pub selected: usize,
    pub state: ListState,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            showing_drives: false,
            entries: Vec::new(),
            selected: 0,
            state: ListState::default(),
        }
    }
}

impl FileBrowser {
    pub fn open(start: impl Into<PathBuf>) -> Self {
        let mut cwd = start.into();
        if !cwd.is_dir() {
            if let Some(parent) = cwd.parent() {
                if parent.is_dir() {
                    cwd = parent.to_path_buf();
                }
            }
        }
        if !cwd.is_dir() {
            cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
        let mut browser = Self {
            cwd,
            showing_drives: false,
            entries: Vec::new(),
            selected: 0,
            state: ListState::default(),
        };
        browser.refresh();
        browser
    }

    pub fn refresh(&mut self) {
        self.entries = if self.showing_drives {
            list_drives()
        } else {
            list_dir(&self.cwd)
        };
        if self.entries.is_empty() {
            self.selected = 0;
            self.state.select(None);
        } else {
            self.selected = self.selected.min(self.entries.len() - 1);
            self.state.select(Some(self.selected));
        }
    }

    pub fn selected_entry(&self) -> Option<&FsEntry> {
        self.entries.get(self.selected)
    }

    pub fn move_sel(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let next = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        self.selected = next;
        self.state.select(Some(self.selected));
    }

    pub fn scroll_sel(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.selected = next;
        self.state.select(Some(self.selected));
    }

    pub fn select_index(&mut self, index: usize) {
        if index < self.entries.len() {
            self.selected = index;
            self.state.select(Some(index));
        }
    }

    /// Enter the selection. `Some(path)` means a file was chosen.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let entry = self.selected_entry().cloned()?;
        if entry.is_dir || entry.is_parent {
            self.enter_dir(entry.path);
            None
        } else {
            Some(entry.path)
        }
    }

    pub fn go_up(&mut self) {
        if self.showing_drives {
            return;
        }
        let from = self.cwd.clone();
        if is_fs_root(&self.cwd) {
            if !list_drives().is_empty() {
                self.showing_drives = true;
                self.selected = 0;
                self.refresh();
                self.select_child(&from);
            }
            return;
        }
        match self.cwd.parent() {
            Some(parent) if parent.is_dir() && parent != self.cwd => {
                self.cwd = parent.to_path_buf();
                self.refresh();
                self.select_child(&from);
            }
            _ => {
                if !list_drives().is_empty() {
                    self.showing_drives = true;
                    self.selected = 0;
                    self.refresh();
                    self.select_child(&from);
                }
            }
        }
    }

    fn enter_dir(&mut self, path: PathBuf) {
        self.showing_drives = false;
        self.cwd = path;
        self.selected = 0;
        self.refresh();
    }

    fn select_child(&mut self, child: &Path) {
        if let Some(idx) = self.entries.iter().position(|e| e.path == child) {
            self.select_index(idx);
        }
    }
}

pub fn list_dir(dir: &Path) -> Vec<FsEntry> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "." || name == ".." {
                continue;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(FsEntry {
                    name,
                    path,
                    is_dir: true,
                    is_parent: false,
                });
            } else if is_xml_file(&path) {
                files.push(FsEntry {
                    name,
                    path,
                    is_dir: false,
                    is_parent: false,
                });
            }
        }
    }
    dirs.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    files.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });

    let mut out = Vec::new();
    if !is_fs_root(dir) {
        if let Some(parent) = dir.parent() {
            if parent.is_dir() && parent != dir {
                out.push(FsEntry {
                    name: "..".into(),
                    path: parent.to_path_buf(),
                    is_dir: true,
                    is_parent: true,
                });
            }
        }
    }
    out.extend(dirs);
    out.extend(files);
    out
}

pub fn is_xml_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
}

fn is_fs_root(path: &Path) -> bool {
    use std::path::Component;
    let mut comps = path.components();
    matches!(
        (comps.next(), comps.next(), comps.next()),
        (Some(Component::Prefix(_)), Some(Component::RootDir), None)
            | (Some(Component::RootDir), None, None)
    )
}

fn list_drives() -> Vec<FsEntry> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .filter_map(|letter| {
                let path = PathBuf::from(format!("{}:\\", letter as char));
                if path.is_dir() {
                    Some(FsEntry {
                        name: format!("{}:\\", letter as char),
                        path,
                        is_dir: true,
                        is_parent: false,
                    })
                } else {
                    None
                }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_xml_and_dirs_skips_other_files() {
        let dir = std::env::temp_dir().join(format!("rimdeck-browse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("keep.xml"), b"<ModsConfigData/>").unwrap();
        fs::write(dir.join("skip.txt"), b"nope").unwrap();
        let entries = list_dir(&dir);
        let names: Vec<_> = entries
            .iter()
            .filter(|e| !e.is_parent)
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"sub"));
        assert!(names.contains(&"keep.xml"));
        assert!(!names.contains(&"skip.txt"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn activate_enters_directory_then_picks_xml() {
        let dir = std::env::temp_dir().join(format!("rimdeck-browse2-{}", std::process::id()));
        let nested = dir.join("lists");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&nested).unwrap();
        let xml = nested.join("modlist.xml");
        fs::write(&xml, b"<ModsConfigData/>").unwrap();
        let mut browser = FileBrowser::open(&dir);
        let idx = browser
            .entries
            .iter()
            .position(|e| e.name == "lists")
            .unwrap();
        browser.select_index(idx);
        assert!(browser.activate().is_none());
        assert_eq!(browser.cwd, nested);
        let idx = browser
            .entries
            .iter()
            .position(|e| e.name == "modlist.xml")
            .unwrap();
        browser.select_index(idx);
        assert_eq!(browser.activate().as_deref(), Some(xml.as_path()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_up_returns_to_parent() {
        let dir = std::env::temp_dir().join(format!("rimdeck-browse3-{}", std::process::id()));
        let nested = dir.join("lists");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&nested).unwrap();
        let mut browser = FileBrowser::open(&nested);
        browser.go_up();
        assert_eq!(browser.cwd, dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unix_root_is_fs_root() {
        assert!(is_fs_root(Path::new("/")));
        assert!(!is_fs_root(Path::new("/tmp")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_root_is_fs_root() {
        assert!(is_fs_root(Path::new(r"C:\")));
        assert!(!is_fs_root(Path::new(r"C:\Windows")));
    }
}
