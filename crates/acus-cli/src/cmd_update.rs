#[derive(clap::Args)]
pub struct Args {
    /// Only report whether a newer release exists (exit 0 if so, 1 if acus is current).
    #[arg(long)]
    check: bool,
    /// Reinstall the latest release even when it is not newer.
    #[arg(long, conflicts_with = "check")]
    force: bool,
}

#[cfg(not(feature = "cmd-update"))]
pub fn run(_: Args) -> anyhow::Result<crate::Outcome> {
    anyhow::bail!(
        "acus was built without `update`\nhint: cargo install --path crates/acus-cli --features cmd-update, or rerun the installer"
    )
}

#[cfg(feature = "cmd-update")]
pub use imp::run;

#[cfg(feature = "cmd-update")]
mod imp {
    use super::Args;
    use crate::Outcome;
    use anyhow::{Context, Result, bail};
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use std::path::Path;
    use std::process::Command;

    const REPO: &str = "LuMiSxh/Acus";
    /// Release archives are a few MB; ureq's default body limit is 10 MB.
    const MAX_DOWNLOAD: u64 = 64 << 20;

    pub fn run(a: Args) -> Result<Outcome> {
        let current = env!("CARGO_PKG_VERSION");
        let exe = std::env::current_exe()?
            .canonicalize()
            .context("cannot locate the running acus binary")?;
        // Windows cannot overwrite a running .exe, so the last update renamed it aside.
        let _ = std::fs::remove_file(exe.with_extension("old"));
        let tag = latest_tag()?;
        let latest = tag.trim_start_matches('v');
        let newer = version(latest) > version(current);
        if a.check {
            return Ok(if newer {
                println!("acus {latest} is available (installed {current}); run acus update");
                Outcome::Found
            } else {
                println!("acus {current} is current");
                Outcome::Empty
            });
        }
        if newer || a.force {
            let Some(target) = target() else {
                bail!(
                    "no release build for {}-{}\nhint: cargo install --git https://github.com/{REPO} acus-cli",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                );
            };
            let (ext, bin) = if cfg!(windows) {
                ("zip", "acus.exe")
            } else {
                ("tar.gz", "acus")
            };
            let url =
                format!("https://github.com/{REPO}/releases/download/{tag}/acus-{target}.{ext}");
            let archive = download(&url)?;
            verify(
                &archive,
                &String::from_utf8_lossy(&download(&format!("{url}.sha256"))?),
            )?;
            let new = extract(&archive, ext, bin)?;
            replace(&exe, &new)?;
            println!("updated {} from {current} to {latest}", exe.display());
        } else {
            println!("acus {current} is current ({})", exe.display());
        }
        // The binary on disk carries the skill of its version.
        let ok = Command::new(&exe)
            .args(["skill", "--install"])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            bail!("cannot reinstall the skill\nhint: run acus skill --install");
        }
        Ok(Outcome::Found)
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .user_agent(concat!("acus/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    }

    fn latest_tag() -> Result<String> {
        let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
        let v: serde_json::Value = agent()
            .get(&url)
            .call()
            .with_context(|| format!("cannot reach {url}"))?
            .body_mut()
            .read_json()?;
        v["tag_name"]
            .as_str()
            .map(String::from)
            .context("the latest release has no tag")
    }

    fn download(url: &str) -> Result<Vec<u8>> {
        agent()
            .get(url)
            .call()
            .with_context(|| format!("cannot download {url}"))?
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()
            .with_context(|| format!("cannot download {url}"))
    }

    /// `0.10.1` > `0.9.3`; non-numeric suffixes are ignored.
    pub(super) fn version(v: &str) -> Vec<u64> {
        v.trim()
            .split('.')
            .map(|p| {
                let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().unwrap_or(0)
            })
            .collect()
    }

    /// Release asset name for this platform, as built by the release workflow.
    fn target() -> Option<&'static str> {
        Some(match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "aarch64-apple-darwin",
            ("macos", "x86_64") => "x86_64-apple-darwin",
            ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
            ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
            ("windows", "x86_64") => "x86_64-pc-windows-msvc",
            _ => return None,
        })
    }

    /// Checks `data` against a `HEX  NAME` checksum line.
    pub(super) fn verify(data: &[u8], sum: &str) -> Result<()> {
        let want = sum
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let got: String = Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if want != got {
            bail!("checksum mismatch: expected {want}, got {got}");
        }
        Ok(())
    }

    /// The file named `bin` from a `tar.gz` or `zip` archive.
    pub(super) fn extract(archive: &[u8], ext: &str, bin: &str) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        if ext == "zip" {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(archive))?;
            z.by_name(bin)
                .with_context(|| format!("{bin} missing from the archive"))?
                .read_to_end(&mut out)?;
            return Ok(out);
        }
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for e in tar.entries()? {
            let mut e = e?;
            if e.path()?.file_name().is_some_and(|n| n == bin) {
                e.read_to_end(&mut out)?;
                return Ok(out);
            }
        }
        bail!("{bin} missing from the archive")
    }

    /// Writes `new` beside `exe` and renames it over; a running Windows binary is first renamed
    /// to `.old`, which the next update removes.
    pub(super) fn replace(exe: &Path, new: &[u8]) -> Result<()> {
        let hint = || {
            format!(
                "cannot write {}\nhint: rerun with permission to write there, or use the installer",
                exe.display()
            )
        };
        let tmp = exe.with_extension("new");
        std::fs::write(&tmp, new).with_context(hint)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        #[cfg(windows)]
        {
            let old = exe.with_extension("old");
            std::fs::rename(exe, &old).with_context(hint)?;
            if let Err(e) = std::fs::rename(&tmp, exe) {
                let _ = std::fs::rename(&old, exe);
                return Err(e).with_context(hint);
            }
        }
        #[cfg(not(windows))]
        std::fs::rename(&tmp, exe).with_context(hint)?;
        Ok(())
    }
}

#[cfg(all(test, feature = "cmd-update"))]
mod tests {
    use super::imp::*;
    use std::io::Write;

    #[test]
    fn compares_versions_numerically() {
        assert!(version("0.10.0") > version("0.9.3"));
        assert!(version("v1.0.0".trim_start_matches('v')) > version("0.99.0"));
        assert_eq!(version("0.5.0"), version("0.5.0\n"));
    }

    #[test]
    fn verifies_checksums() {
        // sha256("abc")
        let sum = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  acus.tar.gz";
        assert!(verify(b"abc", sum).is_ok());
        assert!(verify(b"abd", sum).is_err());
    }

    #[test]
    fn extracts_the_binary_from_both_archive_kinds() {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut h = tar::Header::new_gnu();
        h.set_size(3);
        h.set_mode(0o755);
        h.set_cksum();
        tar.append_data(&mut h, "acus", &b"bin"[..]).unwrap();
        let gz = tar.into_inner().unwrap().finish().unwrap();
        assert_eq!(extract(&gz, "tar.gz", "acus").unwrap(), b"bin");
        assert!(extract(&gz, "tar.gz", "other").is_err());

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("acus.exe", opts).unwrap();
        zip.write_all(b"exe").unwrap();
        let data = zip.finish().unwrap().into_inner();
        assert_eq!(extract(&data, "zip", "acus.exe").unwrap(), b"exe");
    }

    #[test]
    fn replaces_the_binary_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir
            .path()
            .join(if cfg!(windows) { "acus.exe" } else { "acus" });
        std::fs::write(&exe, "old").unwrap();
        replace(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert!(!exe.with_extension("new").exists());
    }
}
