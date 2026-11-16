//! The only ways Droplet hands something to the Windows shell.
//!
//! - `reveal`: select a file in Explorer (asks the running Explorer; starts no process of ours),
//! - `open_own_file`: open a file Droplet itself wrote, with the user's default app,
//! - `open_project_page`: open one of this project's GitHub pages in the default browser.
//!
//! Nothing here runs cmd, PowerShell or an arbitrary program.

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::wide;

pub const REPO_URL: &str = "https://github.com/IsmailMechkene/Droplet";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectPage {
    Releases,
    License,
    Source,
}

impl ProjectPage {
    pub fn from_key(key: &str) -> Option<ProjectPage> {
        match key {
            "releases" => Some(ProjectPage::Releases),
            "license" => Some(ProjectPage::License),
            "source" => Some(ProjectPage::Source),
            _ => None,
        }
    }

    pub fn url(self) -> String {
        match self {
            ProjectPage::Releases => format!("{REPO_URL}/releases"),
            ProjectPage::License => format!("{REPO_URL}/blob/main/LICENSE"),
            ProjectPage::Source => REPO_URL.to_string(),
        }
    }
}

fn shell_open(target: &str) -> bool {
    let (verb, target) = (wide("open"), wide(target));
    let result = unsafe { ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(target.as_ptr()), None, None, SW_SHOWNORMAL) };
    result.0 as isize > 32
}

/// Opens Explorer with `path` selected. The file is not read.
pub fn reveal(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let wide_path = wide(&path.to_string_lossy());
    unsafe {
        let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let item = ILCreateFromPathW(PCWSTR(wide_path.as_ptr()));
        let ok = !item.is_null() && SHOpenFolderAndSelectItems(item, None, 0).is_ok();
        if !item.is_null() {
            ILFree(Some(item));
        }
        if initialised {
            CoUninitialize();
        }
        ok
    }
}

/// Opens a file inside Droplet's own data folder with the default app.
/// Anything outside that folder is refused.
pub fn open_own_file(data_dir: &Path, file_name: &str) -> bool {
    if file_name.contains(['/', '\\', ':']) || file_name.contains("..") {
        return false;
    }
    let path = data_dir.join(file_name);
    path.is_file() && shell_open(&path.to_string_lossy())
}

pub fn open_project_page(page: ProjectPage) -> bool {
    shell_open(&page.url())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_pages_stay_on_the_repo() {
        for page in [ProjectPage::Releases, ProjectPage::License, ProjectPage::Source] {
            assert!(page.url().starts_with(REPO_URL));
        }
        assert_eq!(ProjectPage::from_key("releases"), Some(ProjectPage::Releases));
        assert_eq!(ProjectPage::from_key("https://example.com"), None);
    }

    #[test]
    fn only_plain_file_names_in_the_data_folder_open() {
        let dir = std::env::temp_dir();
        assert!(!open_own_file(&dir, "..\\secret.txt"));
        assert!(!open_own_file(&dir, "C:\\Windows\\notepad.exe"));
        assert!(!open_own_file(&dir, "sub/file.txt"));
        assert!(!open_own_file(&dir, "droplet-file-that-does-not-exist.txt"));
    }
}
