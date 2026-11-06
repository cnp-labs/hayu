use std::fmt;

pub mod atlas;
pub mod cpu;
pub mod scene;

pub mod gl;
#[cfg(feature = "wgpu")]
pub mod wgpu;

pub mod present;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Gl,
    Wgpu,
}

impl Backend {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cpu" => Some(Self::Cpu),
            "gl" => Some(Self::Gl),
            "wgpu" => Some(Self::Wgpu),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gl => "gl",
            Self::Wgpu => "wgpu",
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Usage(String),
    Scene(String),
    Wayland(String),
    Gl(String),
    Wgpu(String),
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "usage: {msg}"),
            Self::Scene(msg) => write!(f, "scene: {msg}"),
            Self::Wayland(msg) => write!(f, "wayland: {msg}"),
            Self::Gl(msg) => write!(f, "gl: {msg}"),
            Self::Wgpu(msg) => write!(f, "wgpu: {msg}"),
            Self::Io(msg) => write!(f, "io: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub struct OffscreenOutput {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub times_ms: Vec<f64>,
    pub renderer: String,
}
