#![forbid(unsafe_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayMode {
    VerifyOnly,
    RenderVideo,
}
