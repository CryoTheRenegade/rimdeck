use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse XML ({path}): {message}")]
    Xml { path: String, message: String },

    #[error("failed to parse settings: {0}")]
    Settings(String),

    #[error("failed to parse rules database: {0}")]
    Rules(String),

    #[error("database download failed: {0}")]
    Fetch(String),

    #[error("failed to launch game: {0}")]
    Launch(String),

    #[error("SteamCMD failed: {0}")]
    Steamcmd(String),

    #[error("workshop search failed: {0}")]
    Workshop(String),

    #[error("GitHub failed: {0}")]
    Github(String),

    #[error("git failed: {0}")]
    Git(String),

    #[error("steam database: {0}")]
    SteamDb(String),

    #[error("circular load-order dependency involving: {0}")]
    Cycle(String),
}

pub type Result<T> = std::result::Result<T, Error>;
