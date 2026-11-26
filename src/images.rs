use std::io;

pub const BROWSER: &str = include_str!("../server/Dockerfile");
pub const AGENT: &str = include_str!("../server/Dockerfile.agent");
pub const APPLICATION: &str = include_str!("../server/Dockerfile.application");
pub const CODEX: &str = include_str!("../server/Dockerfile.codex");

#[derive(Clone)]
pub struct AgentImage {
    pub id: String,
    pub name: String,
    pub dockerfile: String,
}

impl AgentImage {
    pub fn create(name: &str, dockerfile: String, images: &[Self]) -> io::Result<Self> {
        let name = name.trim();
        if !name.eq_ignore_ascii_case("Browser")
            && !name.eq_ignore_ascii_case("Agent")
            && !name.eq_ignore_ascii_case("Application")
            && crate::saved::title(name).is_some()
            && !images
                .iter()
                .any(|image| image.name.eq_ignore_ascii_case(name))
            && !dockerfile.trim().is_empty()
            && dockerfile.len() <= 32768
            && !dockerfile.contains('\0')
        {
            let mut id = [0; 16];
            getrandom::fill(&mut id).map_err(|err| io::Error::other(err.to_string()))?;
            return Ok(Self {
                id: id.iter().map(|b| format!("{b:02x}")).collect(),
                name: name.into(),
                dockerfile,
            });
        }
        Err(io::Error::other(
            "Enter a new, unique image name and a Dockerfile no larger than 32 KiB.",
        ))
    }
}
