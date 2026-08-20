use std::path::PathBuf;

/// Every fallible operation in Roundtable funnels through this type so the UI can
/// show one consistent, human-readable message instead of raw `io::Error` debug text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("io error: {0}")]
    BareIo(#[from] std::io::Error),

    #[error("no ELDEN RING installation is selected")]
    NoGameSelected,

    #[error("{0} is not a folder Roundtable can write to")]
    NotWritable(PathBuf),

    #[error("this file is not a FromSoftware save (expected a BND4 container)")]
    NotASave,

    #[error("save slot {0} is out of range (valid slots are 0-9)")]
    SlotOutOfRange(usize),

    #[error("save file is truncated: needed {needed} bytes but the file is {actual}")]
    SaveTruncated { needed: usize, actual: usize },

    #[error("regulation.bin is not in the expected format: {0}")]
    BadRegulation(String),

    #[error("archive could not be opened: {0}")]
    Archive(String),

    #[error("network request failed: {0}")]
    Network(String),

    #[error("Nexus Mods rejected the request: {0}")]
    Nexus(String),

    #[error("could not parse {what}: {detail}")]
    Parse { what: String, detail: String },

    #[error("{0}")]
    Conflict(String),
}

impl Error {
    pub fn msg(text: impl Into<String>) -> Self {
        Error::Message(text.into())
    }

    pub fn parse(what: impl Into<String>, detail: impl std::fmt::Display) -> Self {
        Error::Parse {
            what: what.into(),
            detail: detail.to_string(),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Error::Network(value.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Error::parse("JSON", value)
    }
}

impl From<toml::de::Error> for Error {
    fn from(value: toml::de::Error) -> Self {
        Error::parse("TOML", value)
    }
}

impl From<toml::ser::Error> for Error {
    fn from(value: toml::ser::Error) -> Self {
        Error::parse("TOML", value)
    }
}

/// Tauri commands must return something serialisable; the UI only ever needs the text.
impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Attaches the offending path to an io error, which turns "access is denied" into
/// "D:\Games\...\mod: access is denied".
pub trait IoContext<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T> IoContext<T> for std::result::Result<T, std::io::Error> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.into(),
            source,
        })
    }
}
