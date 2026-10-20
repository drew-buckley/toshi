#[cfg(feature = "static")]
use std::{env, fs, io, path::Path};

#[cfg(feature = "static")]
const BASE_URL: &str = "https://download.geonames.org/export/dump";

#[cfg(feature = "static")]
const FILES: &[&str] = &[
    "cities15000.zip",
    "cities5000.zip",
    "cities1000.zip",
    "cities500.zip",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(feature = "static")]
    {
        let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
        for file in FILES {
            let dest = Path::new(&out_dir).join(file);
            if dest.exists() {
                continue;
            }
            let url = format!("{BASE_URL}/{file}");
            download(&url, &dest).unwrap_or_else(|e| panic!("failed to download {url}: {e}"));
        }
    }
}

#[cfg(feature = "static")]
fn download(url: &str, dest: &Path) -> io::Result<()> {
    let response = ureq::get(url).call().map_err(io::Error::other)?;
    let mut reader = response.into_body().into_reader();
    let tmp = dest.with_extension("zip.part");
    let mut file = fs::File::create(&tmp)?;
    io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    fs::rename(&tmp, dest)
}
