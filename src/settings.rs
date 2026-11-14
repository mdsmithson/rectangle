use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn path() -> io::Result<PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".config").into_os_string())
        })
        .ok_or_else(|| io::Error::other("No settings directory available"))?;
    Ok(PathBuf::from(dir).join("Rectangle").join("ssh.txt"))
}

pub fn load() -> io::Result<(String, Option<PathBuf>)> {
    load_from(&path()?)
}

pub fn window_path() -> io::Result<PathBuf> {
    Ok(path()?.with_file_name("window.ron"))
}

pub fn tab_volume(host: &str, id: &str) -> Option<u8> {
    let data = fs::read_to_string(path().ok()?.with_file_name("tab-volumes.txt")).ok()?;
    let prefix = format!("{host} {id} ");
    data.lines()
        .find_map(|line| line.strip_prefix(&prefix)?.parse::<u8>().ok())
        .filter(|value| *value <= 100)
}

pub fn save_tab_volume(host: &str, id: &str, volume: u8) -> io::Result<()> {
    let path = path()?.with_file_name("tab-volumes.txt");
    let prefix = format!("{host} {id} ");
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut output = format!("{prefix}{volume}\n");
    for line in data
        .lines()
        .filter(|line| !line.starts_with(&prefix))
        .take(1023)
    {
        output.push_str(line);
        output.push('\n');
    }
    write(&path, &output)
}
pub fn load_secondary() -> io::Result<(String, Option<PathBuf>)> {
    let dir = path()?;
    match load_from(&dir.with_file_name("secondary.txt")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            load_from(&dir.with_file_name("oracle.txt"))
        }
        result => result,
    }
}

pub fn load_startup_page() -> io::Result<String> {
    match fs::read_to_string(path()?.with_file_name("startup-page.txt")) {
        Ok(page) => startup_page(&page),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

pub fn startup_page(input: &str) -> io::Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Enter a website address, such as https://example.com, or leave it blank.",
        )
    };
    if input.len() > 2048
        || input.chars().any(|c| c.is_control() || c.is_whitespace())
        || input.contains('\\')
    {
        return Err(invalid());
    }
    let address = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let page = url::Url::parse(&address).map_err(|_| invalid())?;
    if !matches!(page.scheme(), "https" | "http")
        || !page.has_host()
        || !page.username().is_empty()
        || page.password().is_some()
        || page.as_str().len() > 2048
    {
        return Err(invalid());
    }
    Ok(page.into())
}

pub fn is_bare_site_name(input: &str) -> bool {
    (1..=63).contains(&input.len())
        && !input.starts_with('-')
        && !input.ends_with('-')
        && input
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn load_from(path: &Path) -> io::Result<(String, Option<PathBuf>)> {
    let data = fs::read_to_string(path)?;
    let (host, identity) = data.split_once('\n').unwrap_or((&data, ""));
    let identity = (!identity.is_empty()).then(|| PathBuf::from(identity));
    crate::session::ssh_command(host, identity.as_deref(), "")?;
    Ok((host.to_owned(), identity))
}

pub fn agent_images() -> io::Result<Vec<crate::images::AgentImage>> {
    let dir = path()?.with_file_name("agent-images");
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut images = Vec::new();
    for entry in entries {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if !crate::saved::valid_id(&id)
            || !entry.file_type()?.is_file()
            || entry.metadata()?.len() > 33000
        {
            continue;
        }
        let data = fs::read_to_string(entry.path())?;
        if let Some((name, dockerfile)) = data.split_once('\n')
            && crate::saved::title(name).is_some()
        {
            images.push(crate::images::AgentImage {
                id,
                name: name.into(),
                dockerfile: dockerfile.into(),
            });
        }
    }
    images.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(images)
}

pub fn save_agent_image(image: &crate::images::AgentImage) -> io::Result<()> {
    let path = path()?.with_file_name("agent-images").join(&image.id);
    write(&path, &format!("{}\n{}", image.name, image.dockerfile))
}

pub fn save(
    host: &str,
    identity: &str,
    secondary: &str,
    secondary_key: &str,
    page: &str,
) -> io::Result<()> {
    let dest = path()?;
    if !host.is_empty() {
        validate(host, identity)?;
    }
    if !secondary.is_empty() {
        validate(secondary, secondary_key)?;
    }
    save_optional(&dest, host, identity)?;
    save_optional(
        &dest.with_file_name("secondary.txt"),
        secondary,
        secondary_key,
    )?;
    save_optional(&dest.with_file_name("oracle.txt"), "", "")?;
    write(&dest.with_file_name("startup-page.txt"), page)
}

fn save_optional(dest: &Path, host: &str, key: &str) -> io::Result<()> {
    if !host.is_empty() {
        return save_to(dest, host, key);
    }
    match fs::remove_file(dest) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

fn validate(host: &str, identity: &str) -> io::Result<()> {
    let key = (!identity.is_empty()).then(|| PathBuf::from(identity));
    crate::session::ssh_command(host, key.as_deref(), "")?;
    if identity.contains(['\r', '\n', '\0']) {
        return Err(io::Error::other("Enter a valid SSH key path"));
    }
    Ok(())
}

fn save_to(dest: &Path, host: &str, identity: &str) -> io::Result<()> {
    validate(host, identity)?;
    write(dest, &format!("{host}\n{identity}"))
}

fn write(dest: &Path, value: &str) -> io::Result<()> {
    fs::create_dir_all(dest.parent().unwrap())?;
    let temp = dest.with_extension("tmp");
    fs::write(&temp, value)?;
    fs::rename(temp, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_settings_survive_reopen_and_can_be_replaced() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("settings-test-{}", std::process::id()));
        let path = dir.join("ssh.txt");
        let identity = "C:/my keys/rectangle";
        save_to(&path, "user@server.example", identity).unwrap();
        assert_eq!(
            load_from(&path).unwrap(),
            ("user@server.example".into(), Some(PathBuf::from(identity)))
        );
        save_to(&path, "rectangle-alias", "").unwrap();
        assert_eq!(load_from(&path).unwrap(), ("rectangle-alias".into(), None));
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
