use std::path::Path;

use image::ImageReader;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use tracing::{debug, info};

pub fn init_picker() -> Picker {
    #[cfg(windows)]
    {
        // CSI graphics queries are unreliable in Windows Terminal / conhost.
        // Half-blocks work everywhere and still show Preview.png in color.
        let mut picker = Picker::from_fontsize((10, 20));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        info!("mod preview renderer: halfblocks");
        picker
    }
    #[cfg(not(windows))]
    {
        match Picker::from_query_stdio() {
            Ok(picker) => {
                info!("mod preview renderer: {:?}", picker.protocol_type());
                picker
            }
            Err(e) => {
                tracing::warn!("graphics query failed ({e}); using halfblocks");
                let mut picker = Picker::from_fontsize((10, 20));
                picker.set_protocol_type(ProtocolType::Halfblocks);
                picker
            }
        }
    }
}

pub fn load_protocol(picker: &mut Picker, path: &Path) -> anyhow::Result<StatefulProtocol> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let image = reader.decode()?;
    let image = image.thumbnail(512, 512);
    debug!(path = %path.display(), "loaded mod preview");
    Ok(picker.new_resize_protocol(image))
}

/// Human-readable name of the terminal rimtui is running in.
pub fn terminal_emulator_name() -> String {
    if std::env::var_os("WT_SESSION").is_some() || std::env::var_os("WT_PROFILE_ID").is_some() {
        return "Windows Terminal".into();
    }
    if let Ok(program) = std::env::var("TERM_PROGRAM") {
        return match program.as_str() {
            "vscode" => "Visual Studio Code".into(),
            "cursor" => "Cursor".into(),
            "iTerm.app" => "iTerm2".into(),
            "WezTerm" => "WezTerm".into(),
            "ghostty" => "Ghostty".into(),
            "Apple_Terminal" => "Terminal.app".into(),
            "Hyper" => "Hyper".into(),
            "Alacritty" => "Alacritty".into(),
            "Tabby" => "Tabby".into(),
            other => other.to_string(),
        };
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return "Kitty".into();
    }
    if std::env::var_os("WEZTERM_EXECUTABLE").is_some()
        || std::env::var_os("WEZTERM_PANE").is_some()
    {
        return "WezTerm".into();
    }
    if std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some() {
        return "Ghostty".into();
    }
    if std::env::var_os("ALACRITTY_WINDOW_ID").is_some()
        || std::env::var_os("ALACRITTY_SOCKET").is_some()
    {
        return "Alacritty".into();
    }
    if std::env::var_os("ConEmuPID").is_some() {
        return "ConEmu".into();
    }
    if let Ok(term) = std::env::var("TERM") {
        if term.contains("kitty") {
            return "Kitty".into();
        }
        if term == "foot" || term.starts_with("foot-") {
            return "foot".into();
        }
        if term.contains("alacritty") {
            return "Alacritty".into();
        }
    }
    #[cfg(windows)]
    {
        "this Windows console".into()
    }
    #[cfg(not(windows))]
    {
        "this terminal emulator".into()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn terminal_name_is_nonempty() {
        assert!(!super::terminal_emulator_name().is_empty());
    }
}
